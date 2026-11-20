// @vitest-environment node
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CookieKeyVersion, CookieKeys } from './cookieCrypto';
import type { ElectronCookieDetails } from './cookies';
import {
  planCookieSites,
  runChromeImport,
  type ImportChoices,
  type ImportReport,
  type ImportTargets,
} from './importer';
import { fixtureKeyProvider, type KeyProvider } from './keyProvider';
import type { ChromePreferences } from './profileData';
import {
  buildChromeUserData,
  SENTINELS,
  type ChromeUserDataFixture,
} from '../../../tests/fixtures/chrome-profile/build.mjs';

const NOW_MS = Date.UTC(2026, 10, 19, 12);
const ALL: ImportChoices = { logins: true, bookmarks: true, history: true, settings: true };
let dir: string;
let fixture: ChromeUserDataFixture;

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-chrome-import-'));
  fixture = buildChromeUserData(path.join(dir, 'Chrome'), { now: NOW_MS });
});
afterEach(() => {
  vi.restoreAllMocks();
  fs.rmSync(dir, { recursive: true, force: true });
});

type Sink = {
  targets: ImportTargets;
  jar: ElectronCookieDetails[];
  bookmarks: number[];
  history: number[];
  preferences: ChromePreferences[];
};

function sink(refuse: (details: ElectronCookieDetails) => boolean = () => false): Sink {
  const jar: ElectronCookieDetails[] = [];
  const out: Sink = { jar, bookmarks: [], history: [], preferences: [], targets: null! };
  out.targets = {
    cookies: {
      async set(details) {
        if (refuse(details)) throw new Error('refused');
        jar.push({ ...details });
      },
    },
    addBookmarks: (items) => (out.bookmarks.push(items.length), items.length),
    addHistory: (items) => (out.history.push(items.length), items.length),
    applySettings: (preferences) => {
      out.preferences.push(preferences);
      return ['search engine', 'languages', 'default zoom'];
    },
  };
  return out;
}

function run(
  target: Sink,
  options: Partial<Parameters<typeof runChromeImport>[0]> = {}
): Promise<ImportReport> {
  return runChromeImport({
    profileDir: path.join(fixture.root, 'Default'),
    choices: ALL,
    sites: null,
    keyProvider: fixtureKeyProvider(fixture.password, fixture.platform),
    targets: target.targets,
    nowMs: NOW_MS,
    loginsSupported: true,
    log: () => {},
    tempRoot: dir,
    ...options,
  });
}

const expected = (outcome: string) =>
  fixture.cookies.filter((cookie) => cookie.expect === outcome).map((cookie) => cookie.name);

describe('importing a macOS style profile (BB-22)', () => {
  it('sets exactly the importable cookies, with their flags, and counts the rest', async () => {
    const target = sink();
    const report = await run(target);
    expect(report.logins).toBe('imported');
    expect(target.jar.map((cookie) => cookie.name).sort()).toEqual(expected('imported').sort());
    const byName = Object.fromEntries(target.jar.map((cookie) => [cookie.name, cookie]));
    expect(byName.fixture_session).toEqual({
      url: 'http://site-a.test/',
      name: 'fixture_session',
      value: 'fixture-user',
      path: '/',
      secure: false,
      httpOnly: true,
      sameSite: 'lax',
      expirationDate: Math.floor((NOW_MS + 30 * 86_400_000) / 1000),
    });
    expect(byName.prefs.domain).toBe('.site-a.test');
    expect(byName['__Host-token']).toMatchObject({ secure: true, path: '/', sameSite: 'strict' });
    expect(byName['__Host-token'].domain).toBeUndefined();
    expect(byName['__Secure-id']).toMatchObject({
      secure: true,
      domain: '.site-b.test',
      sameSite: 'no_restriction',
    });
    expect(byName.session_only.expirationDate).toBeUndefined();
    expect(byName.plain_column.value).toBe('fxv-plaintext-column');
    expect(report.cookies.imported).toBe(expected('imported').length);
    expect(report.cookies.sites).toBe(3);
    expect(report.cookies.skipped).toEqual({
      expired: 1,
      partitioned: 1,
      'digest-mismatch': 1,
      'unknown-prefix': 1,
      'app-bound': 1,
      'bad-ciphertext': 1,
      invalid: 2,
    });
    // The manifest gets keys, never values.
    expect(report.cookieKeys).toHaveLength(report.cookies.imported);
    expect(JSON.stringify(report.cookieKeys)).not.toContain('fixture-user');
  });

  it('imports bookmarks, 90 days of history and the settings', async () => {
    const target = sink();
    const report = await run(target);
    expect(report.bookmarks).toBe(5);
    // 90 daily pages inside the window and the dashboard; not the hidden, chrome: or file: rows.
    expect(report.history).toBe(91);
    expect(target.preferences[0]).toEqual({
      searchEngine: { name: 'DuckDuckGo', template: 'https://duckduckgo.com/?q={searchTerms}' },
      acceptLanguages: 'en-US,en,fr-CA',
      defaultZoomFactor: 1.25,
    });
    expect(report.settings).toEqual(['search engine', 'languages', 'default zoom']);
  });

  it('reads an older profile: Cookies at the old location, version 23, no digest', async () => {
    const target = sink();
    const report = await run(target, {
      profileDir: path.join(fixture.root, 'Profile 1'),
      choices: { logins: true, bookmarks: false, history: false, settings: false },
    });
    expect(report.logins).toBe('imported');
    expect(target.jar.map((cookie) => [cookie.name, cookie.value])).toEqual([
      ['personal', 'fxv-personal-v23'],
    ]);
  });

  it('imports only the sites the person chose', async () => {
    const target = sink();
    const report = await run(target, { sites: new Set(['site-b.test']) });
    expect(new Set(target.jar.map((cookie) => cookie.url.split('/')[2]))).toEqual(
      new Set(['site-b.test'])
    );
    expect(report.cookies.sites).toBe(1);
  });

  it('lists sites with counts before any key is asked for', () => {
    const provider = vi.fn();
    const plan = planCookieSites(path.join(fixture.root, 'Default'), dir);
    expect(provider).not.toHaveBeenCalled();
    expect(plan.readable).toBe(true);
    expect(plan.sites.map((site) => site.site)).toEqual(['site-a.test', 'site-b.test', 'ox.ac.uk']);
    expect(plan.total).toBe(fixture.cookies.length - 2);
  });
});

describe('the key (BB-23)', () => {
  it('is asked for only the versions the chosen cookies use, and wiped afterwards', async () => {
    const asked: Array<ReadonlySet<CookieKeyVersion>> = [];
    let handed: CookieKeys = {};
    const provider: KeyProvider = async (needed) => {
      asked.push(new Set(needed));
      const result = await fixtureKeyProvider(fixture.password, 'darwin')(needed);
      if (result.ok) handed = result.keys;
      return result;
    };
    await run(sink(), { keyProvider: provider });
    expect(asked).toEqual([new Set(['v10'])]);
    expect(handed.v10?.every((byte) => byte === 0)).toBe(true);
  });

  it('is never asked for when logins are not chosen, or when nothing is encrypted', async () => {
    const provider = vi.fn<KeyProvider>();
    await run(sink(), {
      keyProvider: provider,
      choices: { logins: false, bookmarks: true, history: true, settings: true },
    });
    const plainOnly = path.join(fixture.root, 'Default', 'Network', 'Cookies');
    const db = new DatabaseSync(plainOnly);
    db.exec("DELETE FROM cookies WHERE name != 'plain_column'");
    db.close();
    const target = sink();
    const report = await run(target, { keyProvider: provider });
    expect(provider).not.toHaveBeenCalled();
    expect(report.logins).toBe('imported');
    expect(target.jar.map((cookie) => cookie.name)).toEqual(['plain_column']);
  });

  it('when access is not allowed: no logins, and the rest still imports', async () => {
    const target = sink();
    const report = await run(target, {
      keyProvider: async () => ({ ok: false, reason: 'not-allowed' }),
    });
    expect(report.logins).toBe('not-allowed');
    expect(target.jar).toEqual([]);
    expect(report.bookmarks).toBe(5);
    expect(report.history).toBe(91);
    expect(report.settings).not.toBeNull();
  });

  it('logins are never read where they cannot be imported (Windows)', async () => {
    const provider = vi.fn<KeyProvider>();
    const report = await run(sink(), { keyProvider: provider, loginsSupported: false });
    expect(report.logins).toBe('unsupported');
    expect(provider).not.toHaveBeenCalled();
    expect(report.bookmarks).toBe(5);
  });
});

describe('a Linux style profile (BB-24)', () => {
  it('decrypts v10 with the fixed password and v11 with the key store password', async () => {
    const linux = buildChromeUserData(path.join(dir, 'google-chrome'), {
      platform: 'linux',
      now: NOW_MS,
    });
    const versions = new Set(linux.cookies.map((cookie) => cookie.version).filter(Boolean));
    expect(versions).toEqual(new Set(['v10', 'v11']));
    const target = sink();
    const report = await runChromeImport({
      profileDir: path.join(linux.root, 'Default'),
      choices: ALL,
      sites: null,
      keyProvider: fixtureKeyProvider(linux.password, 'linux'),
      targets: target.targets,
      nowMs: NOW_MS,
      loginsSupported: true,
      log: () => {},
      tempRoot: dir,
    });
    expect(report.logins).toBe('imported');
    expect(target.jar.map((cookie) => cookie.name).sort()).toEqual(
      linux.cookies
        .filter((cookie) => cookie.expect === 'imported')
        .map((cookie) => cookie.name)
        .sort()
    );
  });
});

describe('reading Chrome files safely', () => {
  it('reads a database Chrome holds locked, through a copy', async () => {
    const live = path.join(fixture.root, 'Default', 'Network', 'Cookies');
    const holder = new DatabaseSync(live);
    holder.exec('PRAGMA locking_mode = EXCLUSIVE; BEGIN EXCLUSIVE;');
    try {
      // The live file itself cannot be read while it is held.
      const reader = new DatabaseSync(live, { timeout: 0 } as never);
      expect(() => reader.prepare('SELECT count(*) FROM cookies').get()).toThrow(/locked/i);
      reader.close();
      const target = sink();
      const report = await run(target);
      expect(report.logins).toBe('imported');
      expect(target.jar.length).toBe(expected('imported').length);
    } finally {
      holder.exec('COMMIT');
      holder.close();
    }
  });

  it('reports an unreadable or damaged cookie file and imports the rest', async () => {
    const live = path.join(fixture.root, 'Default', 'Network', 'Cookies');
    fs.chmodSync(live, 0o000);
    const unreadable = await run(sink());
    expect(unreadable.logins).toBe('unreadable');
    expect(unreadable.bookmarks).toBe(5);
    fs.chmodSync(live, 0o600);
    fs.writeFileSync(live, 'this is not a database');
    const damaged = await run(sink());
    expect(damaged.logins).toBe('unreadable');
    expect(damaged.history).toBe(91);
  });

  it('counts a cookie the browser refuses and carries on', async () => {
    const target = sink((details) => details.name === 'lax_cookie');
    const report = await run(target);
    expect(report.cookies.failedBySite).toEqual({ 'site-b.test': 1 });
    expect(report.cookies.imported).toBe(expected('imported').length - 1);
  });

  it('never opens saved passwords or payment data, and leaves no copy behind', async () => {
    const touched: string[] = [];
    const record = (file: unknown) => touched.push(String(file));
    for (const method of ['copyFileSync', 'readFileSync', 'openSync', 'statSync'] as const) {
      const original = fs[method] as (...args: unknown[]) => unknown;
      vi.spyOn(fs, method).mockImplementation(((...args: unknown[]) => {
        record(args[0]);
        return original.apply(fs, args);
      }) as never);
    }
    await run(sink());
    expect(touched.length).toBeGreaterThan(0);
    for (const sentinel of SENTINELS) {
      expect(touched.filter((file) => file.includes(sentinel))).toEqual([]);
    }
    // The private copies are gone.
    expect(
      fs.readdirSync(dir).filter((name) => name.startsWith('biorouter-chrome-import-'))
    ).toEqual([]);
  });

  it('never puts a value, the password or a key into the log or the report', async () => {
    const lines: string[] = [];
    const report = await run(sink(), {
      log: (message, details) => lines.push(`${message} ${JSON.stringify(details ?? {})}`),
    });
    expect(lines.length).toBeGreaterThanOrEqual(4);
    const logged = lines.join('\n');
    const reported = JSON.stringify(report);
    for (const secret of fixture.secrets) {
      expect(logged).not.toContain(secret);
      expect(reported).not.toContain(secret);
    }
  });
});
