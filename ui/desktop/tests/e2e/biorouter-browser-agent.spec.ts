import { _electron as electron, expect, test, type ElectronApplication } from '@playwright/test';
import { build } from 'esbuild';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
// The fixture site is plain CommonJS so it also runs with `node` by hand.
// eslint-disable-next-line @typescript-eslint/no-require-imports
const { startFixtureSite } = require('./fixtures/biorouter-browser/site.cjs');

/**
 * The agent's half of the Biorouter Browser (BB-12 to BB-15) in a real Electron: the REAL driver
 * (`src/browser/agent/driver.ts`) drives real pages of the loopback fixture site through
 * `webContents.debugger`. What the daemon adds on top (grants, framing, secret scanning) is
 * covered by the Rust tests; what crosses the socket is the request this spec sends.
 *
 * Ungated: no daemon, no model, no network. Run alone:
 *   npx playwright test tests/e2e/biorouter-browser-agent.spec.ts --reporter=list
 */

type Site = {
  url: (role: 'publicA' | 'publicB' | 'intranet', pathname?: string) => string;
  reservedPort: number;
  close: () => Promise<void>;
};

type Result = {
  ok: boolean;
  detail: string;
  data?: Record<string, unknown> & {
    snapshot?: string;
    content?: string;
    tab?: string;
    url?: string;
    title?: string;
    navigated?: boolean;
    needs_act_grant?: boolean;
    dialog?: { type: string; message: string } | null;
    download?: { state: string; filename: string; path?: string; reason?: string } | null;
    tabs?: Array<{ tab: string; active: boolean; url: string }>;
    opened_tabs?: Array<{ tab: string; url: string }>;
    screenshot_path?: string;
    error_kind?: string;
  };
};

let app: ElectronApplication;
let site: Site;
let work: string;

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function call<T = any>(name: string, ...args: unknown[]): Promise<T> {
  return app.evaluate(
    async (_electron, request) =>
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (globalThis as any).__biorouterBrowserTest[request.name](...request.args),
    { name, args }
  );
}

const CHAT = 'chat-agent-1';
function agent(
  op: string,
  args: Record<string, unknown> = {},
  options: Record<string, unknown> = {}
): Promise<Result> {
  return call<Result>('agent', CHAT, op, args, options);
}

/** The reference a snapshot gave a line matching `pattern`. */
function refFor(snapshot: string | undefined, pattern: RegExp): string {
  const line = (snapshot ?? '').split('\n').find((candidate) => pattern.test(candidate));
  const ref = line?.match(/@e\d+/)?.[0];
  if (!ref) throw new Error(`no line matches ${pattern} in:\n${snapshot}`);
  return ref;
}

async function pageTitle(): Promise<string> {
  const result = await agent('tabs', { action: 'list' });
  return result.data?.tabs?.find((tab) => tab.active)?.url ?? '';
}

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  site = (await startFixtureSite()) as Site;
  work = fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-browser-agent-e2e-'));
  const bundle = path.join(work, 'main.cjs');
  await build({
    entryPoints: [path.join(__dirname, 'biorouter-browser.main.ts')],
    outfile: bundle,
    bundle: true,
    platform: 'node',
    format: 'cjs',
    external: ['electron'],
    logLevel: 'warning',
  });
  app = await electron.launch({
    args: [bundle],
    env: {
      ...process.env,
      ELECTRON_RUN_AS_NODE: '',
      BIOROUTER_E2E: '1',
      BIOROUTER_BROWSER_TEST_HOSTS:
        'site-a.test=127.0.0.1:public,site-b.test=127.0.0.1:public,intranet.test=127.0.0.1:private',
      BIOROUTER_BROWSER_TEST_DOWNLOAD_DIR: path.join(work, 'person-downloads'),
      BIOROUTER_BROWSER_TEST_USER_DATA: path.join(work, 'profile'),
    },
  });
  await call('ready');
  await call('reservePort', site.reservedPort);
});

test.afterAll(async () => {
  await app?.close().catch(() => {});
  await site?.close();
  if (work) fs.rmSync(work, { recursive: true, force: true });
});

test('opens a page in a tab the chat panel shows, and snapshots it with references', async () => {
  const opened = await agent(
    'open',
    { url: site.url('publicA', '/agent/form') },
    { grant: 'observe' }
  );
  expect(opened.ok, opened.detail).toBe(true);
  expect(opened.data?.tab).toBe('t1');
  expect(opened.data?.title).toBe('Agent form');
  expect(await call<string[]>('agentViews')).toHaveLength(1);

  const snap = await agent('snapshot', {}, { grant: 'observe' });
  expect(snap.ok, snap.detail).toBe(true);
  const text = snap.data?.snapshot ?? '';
  expect(text).toContain('Page: Agent form');
  expect(text).toMatch(/@e\d+ \[textbox\] "Full name"/);
  expect(text).toMatch(
    /@e\d+ \[combobox\] "Country" .*options="Choose\|Canada\|Mexico\|United States"/
  );
  expect(text).toMatch(/@e\d+ \[checkbox\] "I agree"/);
  expect(text).toMatch(/@e\d+ \[textbox\] "Password" value="\(hidden\)"/);
  expect(text).toMatch(/@e\d+ \[button\] "Send request"/);
  // The host window never took focus from the person (BB-13).
  expect(await call<boolean>('hostFocused')).toBe(false);
});

test('fills, chooses, ticks and submits a form, and the page reads the values', async () => {
  const snap = (await agent('snapshot')).data?.snapshot;
  const fill = (pattern: RegExp, value: string) =>
    agent('fill', { ref: refFor(snap, pattern), value });
  for (const [pattern, value] of [
    [/\[textbox\] "Full name"/, 'Ada Lovelace'],
    [/\[textbox\] "Email"/, 'ada@example.org'],
    [/\[textbox\] "Notes"/, 'Two vials, please.'],
    [/\[textbox\] "Controlled"/, 'react style'],
    [/\[textbox\] "Rich editor"/, 'Edited text'],
  ] as const) {
    const result = await fill(pattern, value);
    expect(result.ok, `${pattern}: ${result.detail}`).toBe(true);
  }
  const date = await agent('fill', { ref: refFor(snap, /"Date"/), value: '2026-11-20' });
  expect(date.ok, date.detail).toBe(true);
  const chose = await agent('select', {
    ref: refFor(snap, /\[combobox\] "Country"/),
    values: ['Mexico'],
  });
  expect(chose.ok, chose.detail).toBe(true);
  expect(chose.detail).toContain('"Mexico"');
  const ticked = await agent('click', { ref: refFor(snap, /\[checkbox\] "I agree"/) });
  expect(ticked.ok, ticked.detail).toBe(true);
  const radio = await agent('click', { ref: refFor(snap, /\[radio\] "Pro"/) });
  expect(radio.ok, radio.detail).toBe(true);

  const submitted = await agent('click', { ref: refFor(snap, /\[button\] "Send request"/) });
  expect(submitted.ok, submitted.detail).toBe(true);
  expect(submitted.data?.navigated).toBe(true);
  expect(submitted.data?.title).toBe('Agent result');

  const read = await agent('read', {}, { grant: 'observe' });
  expect(read.ok, read.detail).toBe(true);
  const table = read.data?.content ?? '';
  for (const row of [
    '| name | Ada Lovelace |',
    '| email | ada@example.org |',
    '| notes | Two vials, please. |',
    '| country | mx |',
    '| consent | yes |',
    '| tier | pro |',
    '| date | 2026-11-20 |',
    // The controlled input's state changed through real input events.
    '| controlled | REACT STYLE |',
    '| editor | Edited text |',
  ]) {
    expect(table).toContain(row);
  }

  // A reference from the form page is stale now, and says so.
  const stale = await agent('click', { ref: refFor(snap, /\[button\] "Send request"/) });
  expect(stale.ok).toBe(false);
  expect(stale.detail).toMatch(
    /is from an earlier page; the tab is now at .*\/agent\/result.*Take a new browser_snapshot/
  );
});

test('never types into a password or verification code field', async () => {
  await agent('open', { url: site.url('publicA', '/agent/form') });
  const snap = (await agent('snapshot')).data?.snapshot;
  for (const pattern of [/\[textbox\] "Password"/, /\[textbox\] "Verification code"/]) {
    const filled = await agent('fill', { ref: refFor(snap, pattern), value: '123456' });
    expect(filled.ok).toBe(false);
    expect(filled.detail).toContain('never types into one');
    const typed = await agent('type', { ref: refFor(snap, pattern), text: '123456' });
    expect(typed.ok).toBe(false);
  }
  const values = await call<string>(
    'run',
    (await call<string[]>('agentViews'))[0],
    "document.getElementById('password').value + '|' + document.getElementById('otp').value"
  );
  expect(values).toBe('|');
});

test('under the reading grant, a plain link is followed and anything else asks to act', async () => {
  await agent('open', { url: site.url('publicA', '/agent/controls') });
  const snap = (await agent('snapshot', {}, { grant: 'observe' })).data?.snapshot;
  const asked = await agent(
    'click',
    { ref: refFor(snap, /"Delete" in:"Alpha 2024"/) },
    { grant: 'observe' }
  );
  expect(asked.ok).toBe(false);
  expect(asked.data?.needs_act_grant).toBe(true);
  // Nothing happened: the click was refused before any input.
  expect(asked.data?.title).toBe('Agent controls');

  await agent('open', { url: site.url('publicA', '/links') }, { grant: 'observe' });
  const links = (await agent('snapshot', {}, { grant: 'observe' })).data?.snapshot;
  const followed = await agent(
    'click',
    { ref: refFor(links, /\[link\] "Same tab: article"/) },
    { grant: 'observe' }
  );
  expect(followed.ok, followed.detail).toBe(true);
  expect(followed.data?.title).toBe('Article on site-a.test');
});

test('refuses hidden, disabled and covered elements with the reason, and scrolls to far ones', async () => {
  await agent('open', { url: site.url('publicA', '/agent/controls') });
  const snap = (await agent('snapshot')).data?.snapshot ?? '';
  expect(snap).toMatch(/\[button\] "Disabled action" \[disabled\]/);
  expect(snap).not.toContain('Hidden action');
  // Unnamed and duplicated controls are told apart by their DOM hints.
  expect(snap).toMatch(/\[button\] "Filter"|\[button\] icon:"Filter"/);
  expect(snap).toMatch(/\[button\] icon:"trash"/);
  expect(snap).toMatch(/\[button\] testid:"close-panel"/);
  expect(snap).toMatch(/\[button\] "Delete" in:"Alpha 2024"/);
  expect(snap).toMatch(/\[button\] "Delete" in:"Beta 2025"/);
  expect(snap).toMatch(/\[button\] "Shadow action"/);

  const disabled = await agent('click', { ref: refFor(snap, /"Disabled action"/) });
  expect(disabled.ok).toBe(false);
  expect(disabled.detail).toContain('is disabled');

  const covered = await agent('click', { ref: refFor(snap, /"Under the banner"/) });
  expect(covered.ok).toBe(false);
  expect(covered.detail).toMatch(/is covered by \[dialog\] "Cookie consent"/);

  const accept = await agent('click', { ref: refFor(snap, /\[button\] "Accept"/) });
  expect(accept.ok, accept.detail).toBe(true);
  expect(accept.data?.title).toBe('accepted cookies');

  const far = await agent('click', { ref: refFor(snap, /"Far down the page"/) });
  expect(far.ok, far.detail).toBe(true);
  expect(far.data?.title).toBe('clicked offscreen');

  const beta = await agent('click', { ref: refFor(snap, /"Delete" in:"Beta 2025"/) });
  expect(beta.ok, beta.detail).toBe(true);
  expect(beta.data?.title).toBe('deleted Beta');

  const shadow = await agent('click', { ref: refFor(snap, /"Shadow action"/) });
  expect(shadow.ok, shadow.detail).toBe(true);
  expect(shadow.data?.title).toBe('clicked shadow');
});

test('reaches into a same-site and a cross-site frame', async () => {
  await agent('open', { url: site.url('publicA', '/agent/frames') });
  // Out-of-process frames attach asynchronously.
  let snap = '';
  await expect
    .poll(async () => {
      snap = (await agent('snapshot')).data?.snapshot ?? '';
      return snap.includes('Inner cross button') && snap.includes('Inner same button');
    })
    .toBe(true);
  expect(snap).toMatch(/frame "Same site frame"/);
  expect(snap).toMatch(/frame "Other site frame" \(http:\/\/site-b\.test/);
  const same = await agent('click', { ref: refFor(snap, /"Inner same button"/) });
  expect(same.ok, same.detail).toBe(true);
  await expect
    .poll(async () => (await agent('tabs', { action: 'list' })).data?.tabs?.[0])
    .toBeTruthy();
  await expect.poll(async () => (await agent('snapshot')).data?.title).toBe('frames: same clicked');
  snap = (await agent('snapshot')).data?.snapshot ?? '';
  const cross = await agent('click', { ref: refFor(snap, /"Inner cross button"/) });
  expect(cross.ok, cross.detail).toBe(true);
  await expect
    .poll(async () => (await agent('snapshot')).data?.title)
    .toBe('frames: cross clicked');
});

test('reads a table as Markdown and one element on its own', async () => {
  await agent('open', { url: site.url('publicA', '/agent/table') }, { grant: 'observe' });
  const page = await agent('read', {}, { grant: 'observe' });
  expect(page.ok, page.detail).toBe(true);
  expect(page.data?.content).toContain('| MBP | 0.91 | Brain |');
  expect(page.data?.content).toContain('- GENE120');
  const short = await agent('read', { max_chars: 500 }, { grant: 'observe' });
  expect(short.data?.truncated).toBe(true);
  const snap = (await agent('snapshot', { interactive_only: false }, { grant: 'observe' })).data
    ?.snapshot;
  const heading = await agent(
    'read',
    { ref: refFor(snap, /\[heading\] "All genes"/) },
    { grant: 'observe' }
  );
  expect(heading.data?.content).toBe('## All genes');
});

test('a page dialog never hangs the agent, and browser_dialog says when none is open', async () => {
  // Measured on Electron 39.8.10: the embedder answers alert and confirm from a browser tab at
  // once (alert returns, confirm reads as declined) and CDP reports no dialog, for the person
  // and the agent alike. So a click that raises one returns, and the page carries on.
  await agent('open', { url: site.url('publicA', '/agent/dialogs') });
  let snap = (await agent('snapshot')).data?.snapshot;
  const alerted = await agent('click', { ref: refFor(snap, /"Show alert"/) });
  expect(alerted.ok, alerted.detail).toBe(true);
  expect(alerted.data?.title).toBe('after alert');
  snap = (await agent('snapshot')).data?.snapshot;
  const confirmed = await agent('click', { ref: refFor(snap, /"Ask to confirm"/) });
  expect(confirmed.ok, confirmed.detail).toBe(true);
  expect(confirmed.data?.title).toBe('not confirmed');
  const none = await agent('dialog', { accept: true });
  expect(none.ok).toBe(false);
  expect(none.detail).toBe('No dialog is open on this page.');
});

test('waits for text, gives up at its timeout, and a cancel stops a slow load', async () => {
  await agent('open', { url: site.url('publicA', '/agent/delayed') }, { grant: 'observe' });
  const ready = await agent(
    'wait',
    { text: 'Ready to go', timeout_ms: 5_000 },
    { grant: 'observe' }
  );
  expect(ready.ok, ready.detail).toBe(true);
  const never = await agent(
    'wait',
    { text: 'never appears', timeout_ms: 600 },
    { grant: 'observe' }
  );
  expect(never.ok).toBe(false);
  expect(never.detail).toContain('was not met');

  const slow = agent(
    'open',
    { url: site.url('publicA', '/agent/slow') },
    { grant: 'observe', requestId: 'slow-1', deadlineMs: 45_000 }
  );
  await new Promise((resolve) => setTimeout(resolve, 600));
  expect(await call<boolean>('agentCancel', 'slow-1')).toBe(true);
  const cancelled = await slow;
  expect(cancelled.ok).toBe(false);
  expect(cancelled.detail).toContain('canceled');
});

test('opens a private network page only for a private chat (BB-06, BB-20)', async () => {
  const publicChat = await agent(
    'open',
    { url: site.url('intranet', '/') },
    { grant: 'observe', tier: 'public' }
  );
  expect(publicChat.ok).toBe(false);
  expect(publicChat.detail).toContain('local or private network address');
  const privateChat = await agent(
    'open',
    { url: site.url('intranet', '/') },
    { grant: 'observe', tier: 'private' }
  );
  expect(privateChat.ok, privateChat.detail).toBe(true);
  await expect
    .poll(
      async () => (await agent('snapshot', {}, { grant: 'observe', tier: 'private' })).data?.title
    )
    .toBe('intranet:ok');
  const daemon = await agent(
    'open',
    { url: `http://127.0.0.1:${site.reservedPort}/` },
    { grant: 'observe', tier: 'private' }
  );
  expect(daemon.ok).toBe(false);
  expect(daemon.detail).toContain('belongs to Biorouter itself');
});

test('captures the viewport and the full page within the size limits', async () => {
  await agent('open', { url: site.url('publicA', '/article') }, { grant: 'observe' });
  const viewport = await agent('screenshot', {}, { grant: 'observe' });
  expect(viewport.ok, viewport.detail).toBe(true);
  const size = await call<{ width: number; height: number }>(
    'imageSize',
    viewport.data?.screenshot_path
  );
  expect(Math.max(size.width, size.height)).toBeLessThanOrEqual(1568);
  expect(size.width).toBeGreaterThan(100);
  const full = await agent('screenshot', { full_page: true }, { grant: 'observe' });
  expect(full.ok, full.detail).toBe(true);
  const fullSize = await call<{ width: number; height: number }>(
    'imageSize',
    full.data?.screenshot_path
  );
  expect(fullSize.height).toBeGreaterThan(size.height);
});

test('downloads only under the act grant, into the chat folder (BB-05)', async () => {
  const downloadDir = path.join(work, 'chat', 'downloads');
  await agent('open', { url: site.url('publicA', '/agent/download-link') }, { grant: 'observe' });
  let snap = (await agent('snapshot', {}, { grant: 'observe' })).data?.snapshot;
  const observed = await agent(
    'click',
    { ref: refFor(snap, /"Download the data"/) },
    { grant: 'observe' }
  );
  expect(observed.data?.download).toMatchObject({
    state: 'cancelled',
    filename: 'fixture-data.csv',
  });
  expect(observed.data?.download?.reason).toContain('permission to act');
  expect(fs.existsSync(path.join(downloadDir, 'fixture-data.csv'))).toBe(false);

  snap = (await agent('snapshot')).data?.snapshot;
  const acted = await agent(
    'click',
    { ref: refFor(snap, /"Download the data"/) },
    { grant: 'act', downloadDir }
  );
  expect(acted.ok, acted.detail).toBe(true);
  expect(acted.data?.download).toMatchObject({ state: 'completed', filename: 'fixture-data.csv' });
  const saved = path.join(downloadDir, 'fixture-data.csv');
  expect(fs.readFileSync(saved, 'utf8')).toContain('MBP,0.91');
  if (process.platform === 'darwin') {
    expect(await call<string | null>('quarantine', saved)).toMatch(/^0081;/);
  }
});

test('works only on a visible tab, and fails with a reason when the panel cannot show it', async () => {
  const views = await call<string[]>('agentViews');
  await call('setAutoShow', false);
  await call('hideView', views[views.length - 1]);
  const hidden = await agent('snapshot', {}, { grant: 'observe', deadlineMs: 10_000 });
  expect(hidden.ok).toBe(false);
  expect(hidden.data?.error_kind).toBe('not_visible');
  await call('setAutoShow', true);
  const again = await agent('snapshot', {}, { grant: 'observe' });
  expect(again.ok, again.detail).toBe(true);
});

test('opens, lists, switches and closes tabs, and sees a tab a page opened', async () => {
  const second = await agent(
    'open',
    { url: site.url('publicA', '/links'), new_tab: true },
    { grant: 'observe' }
  );
  expect(second.ok, second.detail).toBe(true);
  expect(second.data?.tab).toBe('t2');
  const snap = (await agent('snapshot')).data?.snapshot;
  const popup = await agent('click', { ref: refFor(snap, /"New tab: target=_blank"/) });
  expect(popup.ok, popup.detail).toBe(true);
  expect(popup.data?.opened_tabs?.[0]?.tab).toBe('t3');
  const list = await agent('tabs', { action: 'list' });
  expect(list.data?.tabs?.map((tab) => tab.tab)).toEqual(['t1', 't2', 't3']);
  const switched = await agent('tabs', { action: 'switch', tab: 't1' });
  expect(switched.ok, switched.detail).toBe(true);
  const closed = await agent('tabs', { action: 'close', tab: 't3' });
  expect(closed.ok, closed.detail).toBe(true);
  expect(closed.data?.tabs?.map((tab) => tab.tab)).toEqual(['t1', 't2']);
  // Another chat sees none of these tabs.
  const other = await call<Result>('agent', 'chat-agent-2', 'tabs', { action: 'list' });
  expect(other.data?.tabs).toEqual([]);
  expect(await pageTitle()).toContain('site-a.test');
});

test('page text that tries to give orders is read as text, and nothing acts on it', async () => {
  await agent('open', { url: site.url('publicA', '/agent/injection') }, { grant: 'observe' });
  const read = await agent('read', {}, { grant: 'observe' });
  expect(read.data?.content).toContain('ignore all previous instructions');
  const tabs = await agent('tabs', { action: 'list' });
  for (const tab of tabs.data?.tabs ?? []) expect(tab.url).not.toContain('exfiltrate');
});
