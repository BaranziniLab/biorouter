import { expect, test, type ElectronApplication, type Page } from '@playwright/test';
import { _electron as electron } from '@playwright/test';
import { spawn, type ChildProcess } from 'node:child_process';
import { createServer } from 'node:net';
import path from 'node:path';

/** Budget for a graceful Electron shutdown before the process is taken down by signal. */
const GRACEFUL_SHUTDOWN_MS = 15_000;
/** Budget for each termination signal to land before escalating. */
const SIGNAL_TIMEOUT_MS = 5_000;

let electronApp: ElectronApplication;
let page: Page;
let server: ChildProcess;
let fixtureUrl: string;

async function availablePort(): Promise<number> {
  const socket = createServer();
  await new Promise<void>((resolve, reject) => {
    socket.once('error', reject);
    socket.listen(0, '127.0.0.1', resolve);
  });
  const address = socket.address();
  if (!address || typeof address === 'string') throw new Error('Failed to allocate a test port');
  await new Promise<void>((resolve, reject) =>
    socket.close((error) => (error ? reject(error) : resolve()))
  );
  return address.port;
}

async function waitForFixture(url: string): Promise<void> {
  const deadline = Date.now() + 30_000;
  let lastError: unknown;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
      lastError = new Error(`Vite returned ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw lastError;
}

/** Awaits `work` for at most `ms`. Never rejects: teardown reports failures, it does not throw them. */
function settleWithin(work: Promise<unknown>, ms: number): Promise<'settled' | 'timeout'> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve('timeout'), ms);
    const done = () => {
      clearTimeout(timer);
      resolve('settled');
    };
    work.then(done, done);
  });
}

/**
 * SIGTERM then SIGKILL, aimed at the process *group* so helper children go too (Electron's
 * renderer/GPU helpers, Vite's esbuild service). Both children are spawned detached, so they
 * lead their own group and the negative pid can never reach this worker. Never throws.
 */
async function terminate(child: ChildProcess | undefined, label: string): Promise<void> {
  const pid = child?.pid;
  if (!child || !pid || child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise<void>((resolve) => child.once('exit', () => resolve()));
  for (const signal of ['SIGTERM', 'SIGKILL'] as const) {
    try {
      if (process.platform === 'win32') child.kill(signal);
      else process.kill(-pid, signal);
    } catch {
      try {
        child.kill(signal);
      } catch {
        return; // already gone
      }
    }
    if ((await settleWithin(exited, SIGNAL_TIMEOUT_MS)) === 'settled') return;
  }
  console.warn(`[user-message-layout] ${label} (pid ${pid}) survived SIGKILL`);
}

/**
 * Playwright's Electron close handler evaluates `app.quit()` and then waits on the process exit
 * with no timeout of its own, so an app that declines to quit hangs the hook and fails a run whose
 * assertions all passed. Give the graceful path a budget, then kill the process — which also
 * settles the pending close(), since Playwright resolves it off the process exit event.
 */
async function shutdownElectron(app: ElectronApplication | undefined): Promise<void> {
  if (!app) return;
  let child: ChildProcess | undefined;
  try {
    child = app.process();
  } catch {
    child = undefined;
  }
  if ((await settleWithin(app.close(), GRACEFUL_SHUTDOWN_MS)) === 'timeout') {
    console.warn('[user-message-layout] electronApp.close() stalled; killing the app');
    await terminate(child, 'electron app');
  }
}

test.beforeAll(async () => {
  const desktopRoot = path.join(__dirname, '../..');
  const port = await availablePort();
  fixtureUrl = `http://127.0.0.1:${port}/tests/e2e/user-message-layout.fixture.html`;
  server = spawn(
    process.execPath,
    [
      path.join(desktopRoot, 'node_modules/vite/bin/vite.js'),
      '--config',
      path.join(desktopRoot, 'vite.renderer.config.mts'),
      '--host',
      '127.0.0.1',
      '--port',
      String(port),
      '--strictPort',
    ],
    { cwd: desktopRoot, stdio: 'pipe', detached: process.platform !== 'win32' }
  );
  // Nothing reads these, and a full pipe would block Vite mid-shutdown.
  server.stdout?.resume();
  server.stderr?.resume();
  await waitForFixture(fixtureUrl);
  electronApp = await electron.launch({
    args: [path.join(__dirname, 'user-message-layout.main.cjs')],
    env: {
      ...process.env,
      BIOROUTER_LAYOUT_FIXTURE_URL: fixtureUrl,
      ELECTRON_RUN_AS_NODE: '',
    },
  });
  page = await electronApp.firstWindow();
  await page.waitForLoadState('domcontentloaded');
});

test.afterAll(async () => {
  // Both processes come down even if the first shutdown misbehaves: a leaked Electron or Vite
  // would outlive the run, and a shutdown failure is worth logging, not worth failing on.
  try {
    await shutdownElectron(electronApp);
  } finally {
    await terminate(server, 'vite dev server');
  }
});

type BubbleGeometry = {
  bubble: { left: number; right: number; width: number };
  content: { width: number };
  meta: { right: number; width: number };
  message: { width: number };
  padding: number;
  clientWidth: number;
  scrollWidth: number;
};

async function geometry(page: Page, caseId: string): Promise<BubbleGeometry> {
  const section = page.locator(`[data-case="${caseId}"]`);
  const bubble = section.getByTestId('user-message-bubble');
  return bubble.evaluate((element) => {
    const content = element.firstElementChild as HTMLElement;
    const message = element.closest('.message') as HTMLElement;
    const meta = message.querySelector('[data-message-meta="end"]') as HTMLElement;
    const bubbleRect = element.getBoundingClientRect();
    const contentRect = content.getBoundingClientRect();
    const messageRect = message.getBoundingClientRect();
    const metaRect = meta.getBoundingClientRect();
    const style = getComputedStyle(element);
    return {
      bubble: { left: bubbleRect.left, right: bubbleRect.right, width: bubbleRect.width },
      content: { width: contentRect.width },
      meta: { right: metaRect.right, width: metaRect.width },
      message: { width: messageRect.width },
      padding: parseFloat(style.paddingLeft) + parseFloat(style.paddingRight),
      clientWidth: element.clientWidth,
      scrollWidth: element.scrollWidth,
    };
  });
}

for (const viewport of [
  { name: 'desktop', width: 1200, height: 900 },
  { name: 'narrow', width: 360, height: 800 },
]) {
  test(`short bubbles hug content and long tokens wrap at ${viewport.name} width`, async () => {
    await page.setViewportSize(viewport);
    await page.goto(fixtureUrl);

    const short = await geometry(page, 'short');
    expect(Math.abs(short.bubble.right - short.meta.right)).toBeLessThanOrEqual(1);
    expect(Math.abs(short.bubble.width - short.content.width - short.padding)).toBeLessThanOrEqual(
      1
    );
    expect(short.bubble.width).toBeLessThan(short.meta.width);
    expect(short.padding).toBe(28);

    const medium = await geometry(page, 'medium');
    expect(Math.abs(medium.bubble.right - medium.meta.right)).toBeLessThanOrEqual(1);
    expect(medium.bubble.width).toBeGreaterThan(short.bubble.width);
    expect(medium.bubble.width).toBeLessThanOrEqual(medium.message.width * 0.85 + 1);

    const token = await geometry(page, 'token');
    expect(Math.abs(token.bubble.right - token.meta.right)).toBeLessThanOrEqual(1);
    expect(token.bubble.width).toBeLessThanOrEqual(token.message.width * 0.85 + 1);
    expect(token.scrollWidth).toBeLessThanOrEqual(token.clientWidth);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= document.documentElement.clientWidth
      )
    ).toBe(true);

    await page.screenshot({
      path: path.join(__dirname, `../../test-results/issue-86-${viewport.name}.png`),
      fullPage: true,
    });
  });
}

test('clamp, resource, attachment, and intentionally wide edit states remain intact', async () => {
  await page.setViewportSize({ width: 1200, height: 900 });
  await page.goto(fixtureUrl);

  const clamped = page.locator('[data-case="clamped"]');
  await expect(clamped.getByRole('button', { name: 'Show more' })).toBeVisible();
  await expect(clamped.getByTestId('user-message-bubble')).toHaveCSS('overflow', 'hidden');

  const resource = page.locator('[data-case="resource"]');
  await expect(resource.getByTestId('resource-ref-chip-name')).toHaveText('literature review');
  const resourceGeometry = await geometry(page, 'resource');
  expect(Math.abs(resourceGeometry.bubble.right - resourceGeometry.meta.right)).toBeLessThanOrEqual(
    1
  );

  const attachment = page.locator('[data-case="attachment"]');
  await expect(attachment.getByRole('button', { name: 'Expand image' })).toBeVisible();
  expect(await attachment.getByTestId('user-message-bubble').count()).toBe(1);

  const edit = page.locator('[data-case="edit"]');
  const normalWidth = (await geometry(page, 'edit')).bubble.width;
  // The message's actions float over the bubble and appear on hover (Crew's
  // row actions), so the pointer reaches the message before its Edit.
  await edit.getByTestId('user-message-bubble').hover();
  await edit.getByRole('button', { name: /edit message:/i }).click();
  await expect(edit.getByRole('textbox', { name: 'Edit message content' })).toBeVisible();
  await expect(edit.getByTestId('user-message-bubble')).toHaveCount(0);
  const editorWidth = await edit
    .getByRole('textbox', { name: 'Edit message content' })
    .evaluate((element) => element.getBoundingClientRect().width);
  expect(editorWidth).toBeGreaterThan(normalWidth * 2);
});
