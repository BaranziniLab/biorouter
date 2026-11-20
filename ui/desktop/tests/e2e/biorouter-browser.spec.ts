import { _electron as electron, expect, test, type ElectronApplication } from '@playwright/test';
import { build } from 'esbuild';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
// The fixture site is plain CommonJS so it also runs with `node` by hand.
// eslint-disable-next-line @typescript-eslint/no-require-imports
const { startFixtureSite } = require('./fixtures/biorouter-browser/site.cjs');

/**
 * The Biorouter Browser in a real Electron, against a loopback fixture site
 * (docs/desktop-ui/built-in-browser.md, "Electron tests with the loopback fixture site").
 *
 * Ungated: it needs no daemon, no model and no network. `biorouter-browser.main.ts` runs the
 * real main process modules; fixture host names reach loopback through
 * BIOROUTER_BROWSER_TEST_HOSTS, which only an unpackaged app with BIOROUTER_E2E=1 honours.
 *
 * Run alone: npx playwright test tests/e2e/biorouter-browser.spec.ts --reporter=list
 */

type Site = {
  port: number;
  reservedPort: number;
  tlsPort: number | null;
  url: (role: 'publicA' | 'publicB' | 'intranet', pathname?: string) => string;
  log: Array<{ host: string; path: string; ua: string }>;
  close: () => Promise<void>;
};

let app: ElectronApplication;
let site: Site;
let work: string;
let downloads: string;

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function call<T = any>(name: string, ...args: unknown[]): Promise<T> {
  return app.evaluate(
    async (_electron, request) =>
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (globalThis as any).__biorouterBrowserTest[request.name](...request.args),
    { name, args }
  );
}

const title = (viewId: string) => () => call<string>('title', viewId);

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  site = (await startFixtureSite()) as Site;
  work = fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-browser-e2e-'));
  downloads = path.join(work, 'downloads');
  fs.mkdirSync(downloads);
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
      BIOROUTER_BROWSER_TEST_DOWNLOAD_DIR: downloads,
      BIOROUTER_BROWSER_TEST_USER_DATA: path.join(work, 'profile'),
      // The Clipboard API exists only in a secure context; the fixture is plain http.
      BIOROUTER_BROWSER_TEST_SECURE_ORIGINS: new URL(site.url('publicA', '/')).origin,
    },
  });
  await call('ready');
  // The fixture's stand-in daemon is reserved, as main reserves the real daemon's port.
  await call('reservePort', site.reservedPort);
});

test.afterAll(async () => {
  await app?.close().catch(() => {});
  await site?.close();
  if (work) fs.rmSync(work, { recursive: true, force: true });
});

test('opens a page, follows a link in the same tab, and goes back and forward', async () => {
  await call('open', 'links', site.url('publicA', '/links'));
  await expect.poll(title('links')).toBe('Links');
  await call('click', 'links', '#same-tab');
  await expect.poll(title('links')).toBe('Article on site-a.test');
  await expect.poll(async () => (await call('state', 'links'))?.canGoBack).toBe(true);
  await call('control', 'links', 'back');
  await expect.poll(title('links')).toBe('Links');
  await call('control', 'links', 'forward');
  await expect.poll(title('links')).toBe('Article on site-a.test');
});

test('puts a page that sets no background on white, as Chrome does', async () => {
  // The fixture's pages set no colour; on a transparent view they showed the panel through.
  await expect.poll(() => call<number[]>('cornerPixel', 'links')).toEqual([255, 255, 255, 255]);
});

test('keeps a tab’s page and history when the panel hides it (BB-01)', async () => {
  const before = await call('state', 'links');
  await call('release', 'links');
  const again = await call('open', 'links', site.url('publicA', '/links'));
  // The same web contents, still on the article, still able to go back.
  expect(again.sourceRevision.split(':')[0]).toBe(before.sourceRevision.split(':')[0]);
  expect(await call('url', 'links')).toBe(site.url('publicA', '/article'));
  expect(again.canGoBack).toBe(true);
  await call('navigate', 'links', site.url('publicA', '/links'));
  await expect.poll(title('links')).toBe('Links');
});

test('opens target=_blank as a new tab the panel claims (BB-04)', async () => {
  await call('click', 'links', '#new-tab');
  await expect.poll(async () => (await call('sent', 'embedded-browser:opened')).length).toBe(1);
  const [opened] = await call('sent', 'embedded-browser:opened');
  expect(opened.payload).toMatchObject({
    openerViewId: 'links',
    url: site.url('publicA', '/tab-target'),
    foreground: true,
  });
  const viewId = opened.payload.viewId as string;
  expect(await call('claim', viewId)).toBe(true);
  await call('open', viewId, opened.payload.url);
  await expect.poll(title(viewId)).toBe('Tab target');
  await call('destroy', viewId);
});

test('opens a sized window.open as a popup that can talk to its opener (BB-04)', async () => {
  await call('click', 'links', '#popup');
  await expect
    .poll(async () => (await call('windows')).map((w: { url: string }) => w.url))
    .toContain(site.url('publicA', '/popup'));
  await expect.poll(title('links')).toBe('Links: popup said hello');
});

test('blocks a window a page opens without the person, and offers it as a tab', async () => {
  await call('navigate', 'links', site.url('publicA', '/links'));
  await expect.poll(title('links')).toBe('Links');
  const windowsBefore = (await call('windows')).length;
  await call('run', 'links', "window.open('/popup', 'sneaky', 'width=300,height=200'); 1");
  await expect.poll(async () => (await call('state', 'links'))?.notice?.kind).toBe('popup-blocked');
  const notice = (await call('state', 'links')).notice;
  expect(notice.action).toEqual({
    label: 'Open in a new tab',
    kind: 'open-tab',
    url: site.url('publicA', '/popup'),
  });
  expect((await call('windows')).length).toBe(windowsBefore);
});

test('loads sign-in pages in the browser, keeps their login across tabs (BB-03)', async () => {
  const oauthShaped = site.url(
    'publicA',
    `/login?client_id=fixture&redirect_uri=${encodeURIComponent(site.url('publicA', '/dashboard'))}&response_type=code`
  );
  await call('open', 'login', oauthShaped);
  await expect.poll(title('login')).toBe('Sign in');
  expect(await call('url', 'login')).toBe(oauthShaped);
  await call('navigate', 'login', site.url('publicA', '/auth/sso/start'));
  await expect.poll(title('login')).toBe('Sign in');
  await call('click', 'login', '#submit');
  await expect.poll(title('login')).toBe('Dashboard: signed in');
  // The login is the browser profile's, so a new tab is signed in too.
  await call('open', 'second', site.url('publicA', '/dashboard'));
  await expect.poll(title('second')).toBe('Dashboard: signed in');
  expect(await call('cookies', site.url('publicA', '/'))).toContain('fixture_session');
  await call('destroy', 'second');
});

test('loads a page that refuses framing, because a tab is not a frame', async () => {
  await call('navigate', 'login', site.url('publicA', '/xfo'));
  await expect.poll(title('login')).toBe('Refuses framing');
});

test('shows a PDF in the tab instead of downloading it', async () => {
  await call('navigate', 'login', site.url('publicA', '/doc.pdf'));
  await expect.poll(async () => (await call('state', 'login'))?.isLoading).toBe(false);
  expect(await call('url', 'login')).toBe(site.url('publicA', '/doc.pdf'));
  const current = await call('state', 'login');
  expect(current.error).toBeNull();
  expect(current.download ?? null).toBeNull();
});

test('saves a download the person starts, quarantined on macOS (BB-05)', async () => {
  await call('navigate', 'login', site.url('publicA', '/download'));
  await expect.poll(async () => (await call('state', 'login'))?.download?.state).toBe('completed');
  const saved = await call<string>('downloadPath', 'fixture-data.csv');
  expect(fs.readFileSync(saved, 'utf8')).toContain('MBP,0.91');
  const quarantine = await call<string | null>('quarantine', saved);
  if (process.platform === 'darwin') expect(quarantine).toMatch(/^0081;[0-9a-f]+;Biorouter;/);
});

test('presents plain Chrome to sites, never Electron (BB-07)', async () => {
  await call('navigate', 'login', site.url('publicA', '/headers'));
  await expect.poll(title('login')).toBe('Headers');
  const headers = JSON.parse(
    await call<string>('run', 'login', "document.getElementById('headers').textContent")
  );
  expect(headers['user-agent']).toMatch(/Chrome\/\d+\.0\.0\.0 Safari\/537\.36$/);
  expect(headers['user-agent']).not.toMatch(/Electron|Biorouter/i);
  const brands = await call<string>(
    'run',
    'login',
    'JSON.stringify(navigator.userAgentData ? navigator.userAgentData.brands : [])'
  );
  expect(brands).not.toMatch(/Electron/i);
});

test('denies site permissions with a note, but lets Copy buttons work (BB-08)', async () => {
  await call('navigate', 'login', site.url('publicA', '/permissions'));
  await expect.poll(title('login')).toBe('Permissions');
  await call('click', 'login', '#notify');
  await expect.poll(title('login')).toBe('notify:denied');
  await expect
    .poll(async () => (await call('state', 'login'))?.notice)
    .toMatchObject({ kind: 'permission', text: expect.stringContaining('notifications') });
  // Location is refused too (Chromium answers it from the permission check, so it has no note).
  await call('click', 'login', '#geo');
  await expect.poll(title('login')).toBe('geo:denied');
  await call('click', 'login', '#copy');
  await expect.poll(title('login')).toBe('copy:ok');
});

test('cancels HTTP authentication with a note (BB-08)', async () => {
  await call('navigate', 'login', site.url('publicA', '/basic-auth'));
  await expect.poll(async () => (await call('state', 'login'))?.notice?.kind).toBe('auth');
});

test('fails closed on a certificate error and offers the default browser (BB-08)', async () => {
  test.skip(site.tlsPort === null, 'openssl could not make a test certificate');
  await call('navigate', 'login', `https://site-a.test:${site.tlsPort}/`);
  await expect.poll(async () => (await call('state', 'login'))?.errorOffersExternal).toBe(true);
  const current = await call('state', 'login');
  expect(current.error).toMatch(/not secure/);
  expect(await call('title', 'login')).not.toBe('should never be shown');
});

test('lets the person open a local address, and its page reach its own host (BB-06)', async () => {
  await call('open', 'intranet', site.url('intranet', '/'));
  await expect.poll(title('intranet')).toBe('intranet:ok');
  expect((await call('state', 'intranet')).isLocal).toBe(true);
  // A link inside the local page to its own host is followed.
  await call('click', 'intranet', '#intranet-article');
  await expect.poll(title('intranet')).toBe('Article on intranet.test');
});

test('never lets a public page reach a local address (BB-06)', async () => {
  const before = site.log.filter((entry) => entry.path === '/api/ping').length;
  await call('open', 'probe', site.url('publicA', '/local-probe'));
  await expect.poll(title('probe')).toBe('probe:blocked');
  // The request never reached the fixture: refused before the network, not by CORS.
  expect(site.log.filter((entry) => entry.path === '/api/ping').length).toBe(before);
});

test('refuses a public page’s link to a local address, then opens it for the person', async () => {
  await call('navigate', 'links', site.url('publicA', '/links'));
  await expect.poll(title('links')).toBe('Links');
  await call('click', 'links', '#to-intranet');
  await expect.poll(async () => (await call('state', 'links'))?.notice?.kind).toBe('local-blocked');
  const notice = (await call('state', 'links')).notice;
  expect(notice.action).toEqual({
    label: 'Open it',
    kind: 'open-here',
    url: site.url('intranet', '/'),
  });
  // The note's action is the person's own navigation.
  await call('navigate', 'links', notice.action.url);
  await expect.poll(title('links')).toBe('intranet:ok');
});

test('never opens a reserved loopback port, not even for the person (BB-06)', async () => {
  const before = site.log.filter((entry) => entry.host === 'reserved').length;
  await call('navigate', 'probe', `http://127.0.0.1:${site.reservedPort}/`);
  await expect
    .poll(async () => (await call('state', 'probe'))?.notice?.kind)
    .toBe('local-reserved');
  expect(site.log.filter((entry) => entry.host === 'reserved').length).toBe(before);
});

test('maps the browser keys pressed in the page to the bar', async () => {
  await call('navigate', 'links', site.url('publicA', '/links'));
  await expect.poll(title('links')).toBe('Links');
  const modifier = process.platform === 'darwin' ? 'meta' : 'control';
  await call('key', 'links', 'l', [modifier]);
  await expect
    .poll(async () =>
      (await call('sent', 'embedded-browser:shortcut')).map(
        (item: { payload: { action: string } }) => item.payload.action
      )
    )
    .toContain('focus-address');
  await call('key', 'links', 'f', [modifier]);
  await expect
    .poll(async () =>
      (await call('sent', 'embedded-browser:shortcut')).map(
        (item: { payload: { action: string } }) => item.payload.action
      )
    )
    .toContain('find');
});

test('keeps the person’s history for address suggestions, without one time credentials', async () => {
  const found = await call<Array<{ url: string; title: string }>>('suggest', 'Article on');
  expect(found.map((item) => item.url)).toContain(site.url('publicA', '/article'));
  const signedIn = await call<Array<{ url: string }>>('suggest', 'login-submit');
  expect(signedIn).toEqual([]);
  // Every request the browser made went to a fixture host: nothing left the machine.
  const fixtureHosts = new Set(['site-a.test', 'site-b.test', 'intranet.test', 'reserved']);
  expect(site.log.filter((entry) => !fixtureHosts.has(entry.host))).toEqual([]);
});
