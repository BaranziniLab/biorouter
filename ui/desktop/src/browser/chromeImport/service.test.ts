// @vitest-environment node
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createBrowserPreferences, type BrowserPreferences } from '../preferences';
import { createBrowserStore } from '../store';
import type { ElectronCookieDetails } from './cookies';
import { fixtureKeyProvider, type KeyProvider } from './keyProvider';
import { MANIFEST_FILE } from './manifest';
import {
  createChromeImportService,
  validImportRequest,
  type ChromeImportServiceDeps,
} from './service';
import {
  buildChromeUserData,
  type ChromeUserDataFixture,
} from '../../../tests/fixtures/chrome-profile/build.mjs';

const NOW_MS = Date.UTC(2026, 10, 19, 12);
let dir: string;
let fixture: ChromeUserDataFixture;

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-chrome-service-'));
  fixture = buildChromeUserData(path.join(dir, 'Chrome'), { now: NOW_MS });
});
afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

/** A cookie jar that behaves like Electron's for set and remove by url and name. */
function fakeJar() {
  const cookies = new Map<string, ElectronCookieDetails>();
  const id = (details: { url: string; name: string }) =>
    `${new URL(details.url).hostname}\u0000${details.name}`;
  return {
    cookies,
    async set(details: ElectronCookieDetails) {
      cookies.set(id(details), { ...details });
    },
    async remove(url: string, name: string) {
      const host = new URL(url).hostname;
      for (const [key, cookie] of cookies) {
        const domain = cookie.domain?.replace(/^\./, '');
        const matches =
          cookie.name === name &&
          (new URL(cookie.url).hostname === host || (domain && host.endsWith(domain)));
        if (matches) cookies.delete(key);
      }
    },
  };
}

function service(overrides: Partial<ChromeImportServiceDeps> = {}) {
  const stateDir = path.join(dir, 'state');
  const jar = fakeJar();
  const store = createBrowserStore(stateDir, { now: () => NOW_MS });
  const preferences = createBrowserPreferences(stateDir);
  const changed: BrowserPreferences[] = [];
  const lines: string[] = [];
  const deps: ChromeImportServiceDeps = {
    platform: 'darwin',
    dataRoot: fixture.root,
    stateDir,
    cookies: jar,
    store,
    preferences,
    onPreferencesChanged: (next) => changed.push(next),
    keyProvider: fixtureKeyProvider(fixture.password, 'darwin'),
    log: (message, details) => lines.push(`${message} ${JSON.stringify(details ?? {})}`),
    now: () => NOW_MS,
    tempRoot: dir,
    ...overrides,
  };
  return {
    svc: createChromeImportService(deps),
    jar,
    store,
    preferences,
    changed,
    lines,
    stateDir,
  };
}

const everything = {
  profileId: 'Default',
  logins: true,
  bookmarks: true,
  history: true,
  settings: true,
  sites: null,
};

/** The person chose Import in main's own confirmation. */
const yes = async () => true;

describe('the Chrome import service (BB-21 to BB-25)', () => {
  it('offers the import once, until it is dismissed or done', async () => {
    const { svc } = service();
    expect(svc.status()).toMatchObject({
      available: true,
      offerFirstUse: true,
      loginsSupported: true,
      asksForKeyAccess: true,
      imports: [],
    });
    svc.dismissOffer();
    expect(svc.status().offerFirstUse).toBe(false);
    const fresh = service({ stateDir: path.join(dir, 'other') });
    await fresh.svc.run(everything, yes);
    expect(fresh.svc.status().offerFirstUse).toBe(false);
  });

  it('reads nothing until main’s own confirmation says yes (S3)', async () => {
    const asked = vi.fn(fixtureKeyProvider(fixture.password, 'darwin'));
    const { svc, jar, preferences, changed, lines, stateDir } = service({ keyProvider: asked });
    const before = preferences.get();
    const confirm = vi.fn(async () => false);
    const result = await svc.run({ ...everything, sites: ['site-a.test', 'site-b.test'] }, confirm);
    expect(result).toEqual({ ok: false, error: 'The import was canceled.', cancelled: true });
    expect(confirm).toHaveBeenCalledTimes(1);
    const [text] = confirm.mock.calls[0] as unknown as [{ message: string; detail: string }];
    expect(text.message).toBe('Import from Chrome (Work)?');
    for (const line of [
      'Logins (cookies) for 2 sites',
      'Bookmarks',
      'History from the last 90 days',
      'Settings: search engine, languages and default zoom',
      'Saved passwords and payment data are never imported.',
      'macOS will then ask to allow access to Chrome Safe Storage.',
    ]) {
      expect(text.detail).toContain(line);
    }
    expect(asked).not.toHaveBeenCalled();
    expect(jar.cookies.size).toBe(0);
    expect(fs.existsSync(path.join(stateDir, 'imported.json'))).toBe(false);
    expect(preferences.get()).toEqual(before);
    expect(changed).toEqual([]);
    expect(svc.status().imports).toEqual([]);
    expect(lines.some((line) => line.startsWith('cancelled'))).toBe(true);

    // A confirmation that fails is a Cancel, and the import can be asked again.
    const failing = await svc.run(everything, async () => {
      throw new Error('no window');
    });
    expect(failing).toMatchObject({ ok: false, cancelled: true });
    expect((await svc.run(everything, yes)).ok).toBe(true);
  });

  it('says nothing is available when no Chrome is found', () => {
    const { svc } = service({ dataRoot: null });
    expect(svc.status()).toMatchObject({ available: false, offerFirstUse: false });
    expect(svc.profiles()).toEqual([]);
  });

  it('imports into the browser, records a summary, and keeps values out of every file', async () => {
    const { svc, jar, store, preferences, changed, lines, stateDir } = service();
    const result = await svc.run(everything, yes);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.summary).toMatchObject({
      profileName: 'Work',
      logins: 'imported',
      cookies: 10,
      cookieSites: 3,
      skipped: 8,
      failed: 0,
      bookmarks: 5,
      history: 91,
      settings: ['search engine', 'languages', 'default zoom'],
    });
    expect(jar.cookies.get('site-a.test\u0000fixture_session')?.value).toBe('fixture-user');

    const folders = store.importedBookmarkFolders();
    expect(folders.map((folder) => [folder.label, folder.bookmarks.length])).toEqual([
      ['Imported from Chrome (Work)', 5],
    ]);
    expect(store.suggest('fixture-3/').map((item) => item.url)).toEqual([
      'https://pubmed.ncbi.nlm.nih.gov/fixture-3/',
    ]);
    expect(preferences.get()).toEqual({
      searchEngine: { name: 'DuckDuckGo', template: 'https://duckduckgo.com/?q={searchTerms}' },
      acceptLanguages: 'en-US,en,fr-CA',
      defaultZoomFactor: 1.25,
    });
    expect(changed).toHaveLength(1);

    const manifestFile = path.join(stateDir, MANIFEST_FILE);
    expect(fs.statSync(manifestFile).mode & 0o777).toBe(0o600);
    const written = fs
      .readdirSync(stateDir)
      .map((name) => fs.readFileSync(path.join(stateDir, name), 'utf8'))
      .join('\n');
    const said = JSON.stringify([svc.status(), result, lines]);
    for (const secret of fixture.secrets) {
      expect(written).not.toContain(secret);
      expect(said).not.toContain(secret);
    }
    // The renderer is told counts, never cookie names or keys.
    expect(said).not.toContain('fixture_session');
  });

  it('removes exactly what the import brought, and restores the settings', async () => {
    const { svc, jar, store, preferences, changed } = service();
    store.addBookmark('https://example.test/mine', 'Mine');
    await jar.set({
      url: 'https://unrelated.test/',
      name: 'mine',
      value: 'kept',
      path: '/',
      secure: true,
      httpOnly: false,
      sameSite: 'lax',
    });
    preferences.set({ acceptLanguages: 'de' });
    const result = await svc.run(everything, yes);
    if (!result.ok) throw new Error(result.error);
    expect(preferences.get().acceptLanguages).toBe('en-US,en,fr-CA');

    expect(await svc.remove(result.summary.id)).toBe(true);
    expect([...jar.cookies.keys()]).toEqual(['unrelated.test\u0000mine']);
    expect(store.importedBookmarkFolders()).toEqual([]);
    expect(store.suggest('Fixture article')).toEqual([]);
    expect(store.bookmarks().map((bookmark) => bookmark.url)).toEqual([
      'https://example.test/mine',
    ]);
    expect(preferences.get()).toEqual({ acceptLanguages: 'de' });
    expect(changed[changed.length - 1]).toEqual({ acceptLanguages: 'de' });
    expect(svc.status().imports).toEqual([]);
    // Importing answered the first use card; removing the import does not bring it back.
    expect(svc.status().offerFirstUse).toBe(false);
    expect(await svc.remove(result.summary.id)).toBe(false);
  });

  it('removes a cookie by its key even after the site refreshed it', async () => {
    const { svc, jar } = service();
    const result = await svc.run(
      {
        ...everything,
        bookmarks: false,
        history: false,
        settings: false,
      },
      yes
    );
    if (!result.ok) throw new Error(result.error);
    await jar.set({
      url: 'http://site-a.test/',
      name: 'fixture_session',
      value: 'refreshed-by-the-site',
      path: '/',
      secure: false,
      httpOnly: true,
      sameSite: 'lax',
    });
    await svc.remove(result.summary.id);
    expect(jar.cookies.has('site-a.test\u0000fixture_session')).toBe(false);
  });

  it('replaces an earlier import of the same profile instead of adding a second line', async () => {
    const { svc, jar, store } = service();
    const first = await svc.run({ ...everything, sites: ['site-a.test'] }, yes);
    const second = await svc.run({ ...everything, bookmarks: false, sites: ['site-b.test'] }, yes);
    if (!first.ok || !second.ok) throw new Error('import failed');
    const status = svc.status();
    expect(status.imports).toHaveLength(1);
    expect(status.imports[0].cookieSites).toBe(2);
    expect(status.imports[0].bookmarks).toBe(5);
    expect(store.importedBookmarkFolders()).toHaveLength(1);
    await svc.remove(second.summary.id);
    expect(jar.cookies.size).toBe(0);
  });

  it('reports logins as not allowed when access is refused, and imports the rest', async () => {
    const { svc, jar } = service({
      keyProvider: async () => ({ ok: false, reason: 'not-allowed' }),
    });
    const result = await svc.run(everything, yes);
    expect(result.ok && result.summary).toMatchObject({ logins: 'not-allowed', bookmarks: 5 });
    expect(jar.cookies.size).toBe(0);
  });

  it('on Windows, never asks for a key and lists no sites', async () => {
    const keyProvider = vi.fn<KeyProvider>();
    const { svc } = service({ platform: 'win32', keyProvider });
    expect(svc.status()).toMatchObject({ loginsSupported: false, asksForKeyAccess: false });
    expect(svc.sites('Default')).toEqual({ readable: false, sites: [], total: 0 });
    const result = await svc.run(everything, yes);
    expect(result.ok && result.summary.logins).toBe('unsupported');
    expect(keyProvider).not.toHaveBeenCalled();
  });

  it('refuses a profile it did not find itself', async () => {
    const { svc } = service();
    for (const profileId of ['Profile 9', '../escape', '../../etc', '']) {
      expect(await svc.run({ ...everything, profileId }, yes)).toEqual({
        ok: false,
        error: 'That Chrome profile was not found.',
      });
      expect(svc.sites(profileId).readable).toBe(false);
    }
    expect(
      await svc.run(
        {
          ...everything,
          logins: false,
          bookmarks: false,
          history: false,
          settings: false,
        },
        yes
      )
    ).toEqual({
      ok: false,
      error: 'Choose something to import.',
    });
  });

  it('validates what the renderer sends', () => {
    expect(validImportRequest(everything)).toEqual(everything);
    expect(validImportRequest({ ...everything, sites: ['a.test'] })?.sites).toEqual(['a.test']);
    for (const bad of [
      null,
      'Default',
      { ...everything, logins: 'yes' },
      { ...everything, sites: 'all' },
      { ...everything, sites: [1] },
      { ...everything, profileId: 7 },
      { ...everything, sites: new Array(10_001).fill('a.test') },
    ]) {
      expect(validImportRequest(bad)).toBeNull();
    }
  });
});
