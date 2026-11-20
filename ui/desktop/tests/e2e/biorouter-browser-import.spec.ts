import { _electron as electron, expect, test, type ElectronApplication } from '@playwright/test';
import { build } from 'esbuild';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import type { ChromeUserDataFixture } from '../fixtures/chrome-profile/build.mjs';
// The fixture site is plain CommonJS so it also runs with `node` by hand.
// eslint-disable-next-line @typescript-eslint/no-require-imports
const { startFixtureSite } = require('./fixtures/biorouter-browser/site.cjs');

/**
 * Importing from Chrome into the Biorouter Browser, in a real Electron (BB-21 to BB-25,
 * docs/desktop-ui/built-in-browser.md, "Electron tests with the loopback fixture site").
 *
 * A SYNTHETIC Chrome profile, built to Chrome's real formats by
 * `tests/fixtures/chrome-profile/build.mjs`, is imported into a fresh browser profile through the
 * same service main.ts exposes on IPC; the fixture site's dashboard then sees the imported login,
 * and **Remove imported data** signs it out again. The run reads only the fixture
 * (BIOROUTER_BROWSER_TEST_CHROME_DIR under BIOROUTER_E2E=1), and its key comes from the fixture
 * password: no real Chrome profile, Keychain or key store is ever touched.
 *
 * Run alone: npx playwright test tests/e2e/biorouter-browser-import.spec.ts --reporter=list
 */

type Site = {
  port: number;
  reservedPort: number;
  url: (role: 'publicA' | 'publicB' | 'intranet', pathname?: string) => string;
  close: () => Promise<void>;
};

let app: ElectronApplication;
let site: Site;
let work: string;
let fixture: ChromeUserDataFixture;

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
  work = fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-browser-import-e2e-'));
  // In a child Node: Playwright's transform cannot load the builder's ES module.
  fixture = JSON.parse(
    execFileSync(
      process.execPath,
      [
        path.join(__dirname, '../fixtures/chrome-profile/build.mjs'),
        path.join(work, 'chrome-user-data'),
        'darwin',
        '--json',
      ],
      { encoding: 'utf8', env: { ...process.env, NODE_NO_WARNINGS: '1' } }
    )
  ) as ChromeUserDataFixture;
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
      BIOROUTER_DISABLE_KEYRING: 'true',
      BIOROUTER_BROWSER_TEST_HOSTS:
        'site-a.test=127.0.0.1:public,site-b.test=127.0.0.1:public,intranet.test=127.0.0.1:private',
      BIOROUTER_BROWSER_TEST_USER_DATA: path.join(work, 'profile'),
      BIOROUTER_BROWSER_TEST_CHROME_DIR: fixture.root,
      BIOROUTER_BROWSER_TEST_CHROME_PASSWORD: fixture.password,
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

test('finds the synthetic profile and offers the import once', async () => {
  const status = await call('importStatus');
  expect(status).toMatchObject({ available: true, offerFirstUse: true, imports: [] });
  const profiles = await call('importProfiles');
  expect(profiles.map((profile: { id: string }) => profile.id)).toEqual(['Default', 'Profile 1']);
  const sites = await call('importSites', 'Default');
  expect(sites.readable).toBe(true);
  expect(sites.sites.map((entry: { site: string }) => entry.site)).toContain('site-a.test');
});

test('main asks the person itself before reading anything, and Cancel imports nothing (S3)', async () => {
  const dashboard = site.url('publicA', '/dashboard');
  const result = await call(
    'importRun',
    {
      profileId: 'Default',
      logins: true,
      bookmarks: true,
      history: true,
      settings: true,
      sites: null,
    },
    'cancel'
  );
  expect(result).toEqual({ ok: false, error: 'The import was cancelled.', cancelled: true });
  const asked = await call('lastImportConfirmation');
  expect(asked.message).toBe('Import from Chrome (Work)?');
  expect(asked.detail).toContain('Logins (cookies) for every site');
  expect(await call('cookieFlags', 'http://site-a.test/')).toEqual([]);
  expect((await call('importStatus')).imports).toEqual([]);
  await call('open', 'dash-cancel', dashboard);
  await expect.poll(title('dash-cancel')).toBe('Dashboard: signed out');
});

test('an imported login signs the dashboard in, and removing it signs it out', async () => {
  const dashboard = site.url('publicA', '/dashboard');
  await call('open', 'dash', dashboard);
  await expect.poll(title('dash')).toBe('Dashboard: signed out');

  const result = await call('importRun', {
    profileId: 'Default',
    logins: true,
    bookmarks: true,
    history: true,
    settings: true,
    sites: null,
  });
  expect(result.ok).toBe(true);
  expect(result.summary).toMatchObject({
    profileName: 'Work',
    logins: 'imported',
    cookies: 10,
    cookieSites: 3,
    skipped: 8,
    failed: 0,
    bookmarks: 5,
    history: 91,
  });

  await call('control', 'dash', 'reload');
  await expect.poll(title('dash')).toBe('Dashboard: signed in');
  // A new tab shares the one profile (BB-01), so it is signed in too.
  await call('open', 'dash-2', dashboard);
  await expect.poll(title('dash-2')).toBe('Dashboard: signed in');

  // The cookies landed with the flags Chrome had.
  const http = await call('cookieFlags', 'http://site-a.test/');
  const byName = Object.fromEntries(http.map((cookie: { name: string }) => [cookie.name, cookie]));
  expect(byName.fixture_session).toMatchObject({
    hostOnly: true,
    httpOnly: true,
    secure: false,
    sameSite: 'lax',
    session: false,
  });
  expect(byName.prefs).toMatchObject({ hostOnly: false, domain: '.site-a.test' });
  expect(byName.old_cookie).toBeUndefined();
  expect(byName.wrong_digest).toBeUndefined();
  const https = await call('cookieFlags', 'https://site-a.test/');
  expect(https.find((cookie: { name: string }) => cookie.name === '__Host-token')).toMatchObject({
    hostOnly: true,
    secure: true,
    path: '/',
    sameSite: 'strict',
  });
  const b = await call('cookieFlags', 'https://site-b.test/');
  expect(b.find((cookie: { name: string }) => cookie.name === 'session_only')).toMatchObject({
    session: true,
  });
  expect(b.find((cookie: { name: string }) => cookie.name === 'partitioned')).toBeUndefined();

  // Bookmarks in their folder, the search engine, and the languages the next request asks for.
  const bookmarks = await call('bookmarks');
  expect(
    bookmarks.filter((bookmark: { folder?: string }) =>
      bookmark.folder?.startsWith('Imported from Chrome (Work)')
    )
  ).toHaveLength(5);
  expect(await call('searchEngine')).toEqual({
    name: 'DuckDuckGo',
    template: 'https://duckduckgo.com/?q={searchTerms}',
  });
  await call('navigate', 'dash-2', site.url('publicA', '/languages'));
  // Chromium adds the q-values itself.
  await expect.poll(title('dash-2')).toBe('lang:en-US,en;q=0.9,fr-CA;q=0.8');

  const status = await call('importStatus');
  expect(status.offerFirstUse).toBe(false);
  expect(status.imports).toHaveLength(1);

  // Remove imported data (BB-25).
  expect(await call('importRemove', result.summary.id)).toBe(true);
  await call('navigate', 'dash', dashboard);
  await expect.poll(title('dash')).toBe('Dashboard: signed out');
  expect(await call('cookieFlags', 'http://site-a.test/')).toEqual([]);
  expect(await call('searchEngine')).toBeNull();
  expect(
    (await call('bookmarks')).filter((bookmark: { folder?: string }) => bookmark.folder)
  ).toEqual([]);
  expect((await call('importStatus')).imports).toEqual([]);
});

test('no imported value, the password or the key reaches the log or the manifest', async () => {
  const logText = await call<string>('logText');
  expect(logText).toContain('[ChromeImport] logins');
  const stateDir = path.join(work, 'profile', 'biorouter-browser');
  const written = fs
    .readdirSync(stateDir)
    .map((name) => fs.readFileSync(path.join(stateDir, name), 'utf8'))
    .join('\n');
  for (const secret of fixture.secrets) {
    expect(logText).not.toContain(secret);
    expect(written).not.toContain(secret);
  }
});
