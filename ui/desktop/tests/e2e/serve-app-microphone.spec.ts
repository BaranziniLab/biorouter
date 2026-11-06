/**
 * What an app page on the `biorouter serve` origin can do with a microphone
 * grant given to the shell (dictation PR-03, DD-79, SD-16 in
 * docs/deployment/serve-decisions.md).
 *
 * On `serve`, every Agent Drafter app is served on the shell's own origin
 * (SD-14), and a browser grants the microphone to an origin, not to a page. So
 * the daemon sends `Permissions-Policy: microphone=(self)` on the shell and
 * `microphone=(), camera=(), display-capture=()` on every app document. This
 * spec starts a real `biorouter serve`, grants the microphone to its origin the
 * way a person who allowed dictation for good would have, and checks in
 * Chromium that the shell records while an app page, an app file opened on its
 * own, the launch bounce and the figure frame on the same origin cannot.
 *
 * The header governs those documents and the frames they hold, not the
 * origin's grant. Two residuals follow, and SD-16 records both:
 *
 * - In Chromium, a same-origin popup an app page opens does not inherit the
 *   app's policy, so the page records through it. The second test pins that
 *   as it is today, labelled SD-16 residual, so a mitigation has to flip it on
 *   purpose. No response header can close the `about:blank` case; per app
 *   origins do.
 * - Firefox and WebKit ignore `Permissions-Policy`, so there the app page
 *   itself can use the origin's grant. The last two tests name that in the
 *   report rather than run, because no header can change it.
 *
 * It needs a `biorouter` with its `biorouterd` beside it, built from the tree
 * under test, and a Chromium. By default it uses `target/debug/biorouter` and
 * Playwright's own Chromium, and skips when either is missing. Set
 * `BIOROUTER_SERVE_BIN` to require both (CI), and `BIOROUTER_PW_CHANNEL` to use
 * an installed browser instead (`chrome` on a machine that has Google Chrome
 * and no Playwright download); a channel that is named must start.
 *
 *   npx playwright test tests/e2e/serve-app-microphone.spec.ts --reporter=list
 */

import { test, expect, chromium } from '@playwright/test';
import type { Browser, BrowserContext, Page, Response } from '@playwright/test';
import { spawn } from 'child_process';
import type { ChildProcess } from 'child_process';
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'fs';
import { createServer } from 'net';
import { tmpdir } from 'os';
import { join, resolve } from 'path';

const APP_ID = 'mic-probe';
const TOKEN = 'pr03-microphone-probe-token';
const APP_POLICY = 'microphone=(), camera=(), display-capture=()';
const SHELL_POLICY = 'microphone=(self)';

const REQUIRED_BIN = process.env.BIOROUTER_SERVE_BIN;
const SERVE_BIN = REQUIRED_BIN ?? resolve(__dirname, '../../../../target/debug/biorouter');
const CHANNEL = process.env.BIOROUTER_PW_CHANNEL || undefined;

/** What `getUserMedia({ audio: true })` gave the page. */
type Capture = { ok: true; tracks: number } | { ok: false; name: string; message: string };

/** What an app page's own script got from a popup it opened. */
type PopupCapture =
  | { opened: false }
  | { opened: true; href: string; allows: boolean | null; capture: Capture };

let root = '';
let origin = '';
let serve: ChildProcess | undefined;
let serveLog = '';
let browser: Browser | undefined;
let context: BrowserContext | undefined;
let page: Page;
let skipReason = '';

async function freePort(): Promise<number> {
  return new Promise((resolvePort, reject) => {
    const server = createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      const port = typeof address === 'object' && address ? address.port : 0;
      server.close(() => resolvePort(port));
    });
  });
}

/**
 * A static app on disk, where the daemon's store reads it
 * (`$BIOROUTER_PATH_ROOT/config/agent_drafter/<id>/`), with one HTML file
 * under `assets/` so `with_app_file_csp` is exercised as well as the page.
 */
function writeFixtures(): string {
  const dir = mkdtempSync(join(tmpdir(), 'br-serve-mic-'));
  const app = join(dir, 'config', 'agent_drafter', APP_ID);
  mkdirSync(join(app, 'assets'), { recursive: true });
  writeFileSync(
    join(app, 'manifest.json'),
    JSON.stringify({ id: APP_ID, title: 'Microphone probe', kind: 'static', entry: 'index.html' })
  );
  writeFileSync(
    join(app, 'index.html'),
    '<!doctype html><html><head><title>Microphone probe</title></head>' +
      '<body><p id="probe">app page</p></body></html>'
  );
  writeFileSync(
    join(app, 'assets', 'probe.html'),
    '<!doctype html><html><head><title>Asset probe</title></head><body>asset</body></html>'
  );
  // The shell only has to be a document of the origin; the headers under test
  // are the daemon's, not the bundle's. `serve` requires an `index.html`.
  const web = join(dir, 'web');
  mkdirSync(web, { recursive: true });
  writeFileSync(
    join(web, 'index.html'),
    '<!doctype html><html><head><title>Biorouter</title></head><body>shell</body></html>'
  );
  return dir;
}

async function waitForServe(): Promise<void> {
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    if (serve?.exitCode !== null && serve?.exitCode !== undefined) {
      throw new Error(`biorouter serve exited with ${serve.exitCode}:\n${serveLog}`);
    }
    try {
      const res = await fetch(`${origin}/status`);
      if (res.status > 0) return;
    } catch {
      // Not listening yet.
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`biorouter serve did not answer within 90s:\n${serveLog}`);
}

/** Ask for the microphone in the page's top document, and stop what it got. */
async function capture(target: Page): Promise<Capture> {
  return target.evaluate(async (): Promise<Capture> => {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const tracks = stream.getAudioTracks().length;
      stream.getTracks().forEach((track) => track.stop());
      return { ok: true, tracks };
    } catch (error) {
      const e = error as DOMException;
      return { ok: false, name: e.name, message: e.message };
    }
  });
}

/** Chromium's own answer, where it exposes one, to "may this document use the microphone". */
async function policyAllowsMicrophone(target: Page): Promise<boolean | null> {
  return target.evaluate(() => {
    const doc = document as Document & {
      featurePolicy?: { allowsFeature(feature: string): boolean };
      permissionsPolicy?: { allowsFeature(feature: string): boolean };
    };
    const policy = doc.permissionsPolicy ?? doc.featurePolicy;
    return policy ? policy.allowsFeature('microphone') : null;
  });
}

/**
 * From the current page's own script, the way code an agent wrote into an app
 * would: open `url` in a popup, wait for it to load, and ask for the
 * microphone through the popup's `navigator`. Playwright's evaluate carries a
 * user gesture, as any click in a real app would.
 */
async function captureThroughPopup(target: Page, url: string): Promise<PopupCapture> {
  return target.evaluate(async (address): Promise<PopupCapture> => {
    const popup = window.open(address);
    if (!popup) return { opened: false };
    try {
      for (let i = 0; i < 100; i++) {
        const settled =
          popup.document.readyState === 'complete' &&
          (address === 'about:blank' || popup.location.href !== 'about:blank');
        if (settled) break;
        await new Promise((r) => setTimeout(r, 100));
      }
      const doc = popup.document as Document & {
        featurePolicy?: { allowsFeature(feature: string): boolean };
        permissionsPolicy?: { allowsFeature(feature: string): boolean };
      };
      const policy = doc.permissionsPolicy ?? doc.featurePolicy;
      const allows = policy ? policy.allowsFeature('microphone') : null;
      let result: Capture;
      try {
        const stream = await popup.navigator.mediaDevices.getUserMedia({ audio: true });
        result = { ok: true, tracks: stream.getAudioTracks().length };
        stream.getTracks().forEach((track) => track.stop());
      } catch (error) {
        const e = error as DOMException;
        result = { ok: false, name: e.name, message: e.message };
      }
      return { opened: true, href: popup.location.href, allows, capture: result };
    } finally {
      popup.close();
    }
  }, url);
}

function policyOf(response: Response | null): string | undefined {
  return response?.headers()['permissions-policy'];
}

/** Open the shell the way `serve`'s link does: a bounce page trades the token for a cookie. */
async function openShell(): Promise<Response | null> {
  await page.goto(`${origin}/?t=${TOKEN}`);
  await page.waitForURL(`${origin}/`);
  return page.goto(`${origin}/`);
}

/** Ask for an app's launch link with the daemon secret the shell carries, as the interface does. */
async function launchPath(): Promise<string> {
  const secret = await page.evaluate(
    () =>
      (window as unknown as { __BIOROUTER_HEADLESS_CONFIG__?: { secretKey?: string } })
        .__BIOROUTER_HEADLESS_CONFIG__?.secretKey
  );
  expect(secret, 'the shell carries the daemon secret').toMatch(/^[0-9a-f]{64}$/);
  const launch = await context!.request.post(`${origin}/apps/${APP_ID}/launch`, {
    headers: { 'X-Secret-Key': secret! },
  });
  expect(launch.status(), await launch.text()).toBe(200);
  const { path } = (await launch.json()) as { path: string };
  expect(path).toMatch(new RegExp(`^/apps/${APP_ID}/\\?t=[0-9a-f]+$`));
  return path;
}

test.describe('serve: an app page and the shell origin microphone grant', () => {
  test.setTimeout(180_000);

  test.beforeAll(async () => {
    if (!existsSync(SERVE_BIN)) {
      if (REQUIRED_BIN) throw new Error(`BIOROUTER_SERVE_BIN names no file: ${REQUIRED_BIN}`);
      skipReason = `no biorouter at ${SERVE_BIN}; build it, or set BIOROUTER_SERVE_BIN`;
      return;
    }

    // The browser first, so a machine without one skips before a daemon starts.
    try {
      browser = await chromium.launch({
        channel: CHANNEL,
        // A fake microphone, so the shell's capture shows the grant works.
        // Deliberately NOT --use-fake-ui-for-media-stream, which would accept
        // every prompt: the only grant is the one given to the origin below.
        args: ['--use-fake-device-for-media-stream'],
      });
    } catch (error) {
      // CI names the binary, and a person who names a channel asked for that
      // browser: both must run. A developer machine that has built `biorouter`
      // but never downloaded Playwright's Chromium skips, as the Electron
      // specs beside this one need no browser download.
      if (REQUIRED_BIN || CHANNEL) throw error;
      const first = String((error as Error).message ?? error).split('\n')[0];
      skipReason =
        `Playwright's Chromium did not start (${first}). Run npx playwright install ` +
        'chromium, or set BIOROUTER_PW_CHANNEL=chrome to use an installed Google Chrome';
      return;
    }

    root = writeFixtures();
    const port = await freePort();
    origin = `http://127.0.0.1:${port}`;
    serve = spawn(SERVE_BIN, ['serve', '--port', String(port), '--web-dir', join(root, 'web')], {
      env: {
        ...process.env,
        BIOROUTER_PATH_ROOT: root,
        BIOROUTER_DISABLE_KEYRING: 'true',
        // From the environment rather than `--token`, which `ps` shows.
        BIOROUTER_BROWSER_TOKEN: TOKEN,
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    serve.stdout?.on('data', (chunk) => (serveLog += chunk.toString()));
    serve.stderr?.on('data', (chunk) => (serveLog += chunk.toString()));
    await waitForServe();

    context = await browser.newContext();
    await context.grantPermissions(['microphone'], { origin });
    page = await context.newPage();
  });

  test.afterAll(async () => {
    await context?.close();
    await browser?.close();
    if (serve && serve.exitCode === null) {
      const exited = new Promise((r) => serve?.once('exit', r));
      serve.kill('SIGTERM');
      await Promise.race([exited, new Promise((r) => setTimeout(r, 15_000))]);
      if (serve.exitCode === null) serve.kill('SIGKILL');
    }
    if (root) rmSync(root, { recursive: true, force: true });
  });

  test('Chromium: the shell records and every app document on its origin is refused', async () => {
    test.skip(!browser || !serve, skipReason);

    const shell = await openShell();
    expect(shell?.status()).toBe(200);
    expect(policyOf(shell)).toBe(SHELL_POLICY);

    // The control: on this origin, under this grant, the shell gets audio. So
    // a refusal below is the app's header, not a missing grant or device.
    expect(await capture(page)).toEqual({ ok: true, tracks: 1 });
    const shellAllows = await policyAllowsMicrophone(page);
    if (shellAllows !== null) expect(shellAllows).toBe(true);

    const path = await launchPath();
    const bounce = await page.goto(`${origin}${path}`);
    expect(bounce?.status()).toBe(200);
    expect(policyOf(bounce), 'the launch bounce').toBe(APP_POLICY);
    await page.waitForURL(`${origin}/apps/${APP_ID}/`);

    // The app page.
    const appPage = await page.goto(`${origin}/apps/${APP_ID}/`);
    expect(appPage?.status()).toBe(200);
    expect(policyOf(appPage), 'the app page').toBe(APP_POLICY);
    await expect(page.locator('#probe')).toHaveText('app page');
    const refused = await capture(page);
    expect(refused.ok, JSON.stringify(refused)).toBe(false);
    expect(refused.ok ? '' : refused.name).toBe('NotAllowedError');
    const appAllows = await policyAllowsMicrophone(page);
    if (appAllows !== null) expect(appAllows).toBe(false);

    // An HTML file under `assets/`, opened on its own.
    const asset = await page.goto(`${origin}/apps/${APP_ID}/assets/probe.html`);
    expect(asset?.status()).toBe(200);
    expect(policyOf(asset), 'an assets/ document').toBe(APP_POLICY);
    const assetCapture = await capture(page);
    expect(assetCapture.ok ? '' : assetCapture.name, JSON.stringify(assetCapture)).toBe(
      'NotAllowedError'
    );

    // The figure frame. Its sandbox gives it an opaque origin, which holds no
    // grant anyway, so only its header is checked here.
    const frame = await page.goto(`${origin}/apps/${APP_ID}/figure-frame`);
    expect(frame?.status()).toBe(200);
    expect(policyOf(frame), 'the figure frame').toBe(APP_POLICY);

    // And the grant itself is untouched: back on the shell, it still records.
    await page.goto(`${origin}/`);
    expect(await capture(page)).toEqual({ ok: true, tracks: 1 });
  });

  test('Chromium: an app page records through a same-origin popup (SD-16 residual)', async () => {
    test.skip(!browser || !serve, skipReason);
    // This pins a hole as it is today, measured in Chrome 157 on 2026-11-05. A
    // popup is a new top-level document: it takes the policy of the response
    // that made it, or none for `about:blank` and for a document this origin
    // serves without the header, never the opener's. If a change makes this
    // test fail, it closed the residual: flip the expectations on purpose and
    // update SD-16 in docs/deployment/serve-decisions.md in the same change.
    await openShell();
    const path = await launchPath();
    await page.goto(`${origin}${path}`);
    await page.waitForURL(`${origin}/apps/${APP_ID}/`);
    await expect(page.locator('#probe')).toHaveText('app page');

    // The page itself is refused, as the first test shows.
    const own = await capture(page);
    expect(own.ok ? '' : own.name, JSON.stringify(own)).toBe('NotAllowedError');

    // A blank popup: no response, so no header could ever reach it.
    expect(await captureThroughPopup(page, 'about:blank')).toEqual({
      opened: true,
      href: 'about:blank',
      allows: true,
      capture: { ok: true, tracks: 1 },
    });

    // A document of this origin served without the header.
    const status = await captureThroughPopup(page, '/status');
    expect(status).toEqual({
      opened: true,
      href: `${origin}/status`,
      allows: true,
      capture: { ok: true, tracks: 1 },
    });
  });

  for (const engine of ['Firefox', 'WebKit']) {
    test(`${engine}: the documented residual (SD-16)`, async () => {
      test.skip(
        true,
        `${engine} ignores Permissions-Policy, so an app page on the serve origin can use a ` +
          'microphone grant the person gave the shell, in its own page as well as through a ' +
          'popup. No header changes that; SD-16 records it, and per app origins close it.'
      );
    });
  }
});
