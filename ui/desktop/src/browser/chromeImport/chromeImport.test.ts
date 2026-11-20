// @vitest-environment node
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  cookieValueVersion,
  decryptCookieValue,
  deriveChromeKey,
  encryptCookieValue,
  LINUX_ITERATIONS,
  LINUX_V10_PASSWORD,
  MACOS_ITERATIONS,
} from './cookieCrypto';
import {
  CHROME_EPOCH_OFFSET_SECONDS,
  cookieSite,
  cookieSiteCounts,
  toElectronCookie,
  type ChromeCookieRow,
} from './cookies';
import { chromeMicrosToUnixMs, readChromeBookmarks, readChromePreferences } from './profileData';
import {
  chromeCookiesFile,
  chromeDataRoot,
  chromeProfileDir,
  chromeProfileFiles,
  readChromeProfiles,
} from './profiles';
import { buildChromeUserData } from '../../../tests/fixtures/chrome-profile/build.mjs';

const NOW_MS = Date.UTC(2026, 10, 19, 12);
const NOW_S = Math.floor(NOW_MS / 1000);
let dir: string;

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-chrome-unit-'));
});
afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

describe('Chrome cookie encryption (BB-22)', () => {
  it('derives the keys Chrome derives', () => {
    // Chrome's fixed Linux v10 key, the value every Chromium cookie tool agrees on.
    expect(deriveChromeKey(LINUX_V10_PASSWORD, LINUX_ITERATIONS).toString('hex')).toBe(
      'fd621fe5a2b402539dfa147ca9272778'
    );
    expect(deriveChromeKey('x', MACOS_ITERATIONS)).toHaveLength(16);
    expect(deriveChromeKey('x', MACOS_ITERATIONS).equals(deriveChromeKey('x', 1))).toBe(false);
  });

  it('decrypts v10 with and without the host digest, and checks the digest', () => {
    const key = deriveChromeKey('fixture', MACOS_ITERATIONS);
    const withDigest = encryptCookieValue('hello', key, {
      version: 'v10',
      hostKey: '.example.test',
      digestPrefixed: true,
    });
    expect(cookieValueVersion(withDigest)).toBe('v10');
    expect(
      decryptCookieValue(
        withDigest,
        { v10: key },
        { hostKey: '.example.test', digestPrefixed: true }
      )
    ).toEqual({ ok: true, value: 'hello' });
    // The same bytes under another host: the digest no longer matches, so nothing is imported.
    expect(
      decryptCookieValue(withDigest, { v10: key }, { hostKey: 'other.test', digestPrefixed: true })
    ).toEqual({ ok: false, reason: 'digest-mismatch' });

    const plain = encryptCookieValue('hello', key, {
      version: 'v10',
      hostKey: 'example.test',
      digestPrefixed: false,
    });
    expect(
      decryptCookieValue(plain, { v10: key }, { hostKey: 'example.test', digestPrefixed: false })
    ).toEqual({ ok: true, value: 'hello' });
  });

  it('names every value it cannot decrypt', () => {
    const key = deriveChromeKey('fixture', 1);
    const good = encryptCookieValue('v', key, {
      version: 'v11',
      hostKey: 'a.test',
      digestPrefixed: false,
    });
    const options = { hostKey: 'a.test', digestPrefixed: false };
    expect(decryptCookieValue(good, {}, options)).toEqual({ ok: false, reason: 'no-key' });
    expect(decryptCookieValue(good, { v11: deriveChromeKey('wrong', 1) }, options).ok).toBe(false);
    const appBound = Buffer.concat([Buffer.from('v20'), good.subarray(3)]);
    expect(decryptCookieValue(appBound, { v11: key }, options)).toEqual({
      ok: false,
      reason: 'app-bound',
    });
    expect(decryptCookieValue(Buffer.from('xyz123'), { v11: key }, options)).toEqual({
      ok: false,
      reason: 'unknown-prefix',
    });
    expect(decryptCookieValue(good.subarray(0, good.length - 3), { v11: key }, options)).toEqual({
      ok: false,
      reason: 'bad-ciphertext',
    });
    const binary = Buffer.concat([
      Buffer.from('v11'),
      (() => {
        const cipher = crypto.createCipheriv('aes-128-cbc', key, Buffer.alloc(16, 0x20));
        return Buffer.concat([cipher.update(Buffer.from([0xff, 0xfe, 0xfd])), cipher.final()]);
      })(),
    ]);
    expect(decryptCookieValue(binary, { v11: key }, options)).toEqual({
      ok: false,
      reason: 'not-text',
    });
  });
});

function row(overrides: Partial<ChromeCookieRow>): ChromeCookieRow {
  return {
    hostKey: 'www.example.test',
    name: 'n',
    plainValue: '',
    encryptedValue: new Uint8Array(),
    path: '/',
    expiresSeconds: NOW_S + CHROME_EPOCH_OFFSET_SECONDS + 3600,
    hasExpires: true,
    secure: false,
    httpOnly: false,
    sameSite: -1,
    partitioned: false,
    ...overrides,
  };
}

describe('converting a Chrome cookie for session.cookies.set', () => {
  it('keeps a host only cookie host only and gives a domain cookie its domain', () => {
    const host = toElectronCookie(row({ hostKey: 'www.example.test' }), 'v', NOW_S);
    expect(host.ok && host.details).toMatchObject({
      url: 'http://www.example.test/',
      path: '/',
      expirationDate: NOW_S + 3600,
    });
    expect(host.ok && 'domain' in host.details).toBe(false);

    const domain = toElectronCookie(
      row({ hostKey: '.example.test', secure: true, path: '/app' }),
      'v',
      NOW_S
    );
    expect(domain.ok && domain.details).toMatchObject({
      url: 'https://example.test/app',
      domain: '.example.test',
      path: '/app',
      secure: true,
    });
    expect(domain.ok && domain.key).toEqual({
      url: 'https://example.test/app',
      name: 'n',
      domain: '.example.test',
      path: '/app',
    });
  });

  it('gives __Host- and __Secure- cookies what their prefix requires', () => {
    const hostPrefixed = toElectronCookie(
      row({ name: '__Host-t', path: '/deep', secure: false }),
      'v',
      NOW_S
    );
    expect(hostPrefixed.ok && hostPrefixed.details).toMatchObject({
      url: 'https://www.example.test/',
      secure: true,
      path: '/',
    });
    expect(hostPrefixed.ok && hostPrefixed.details.domain).toBeUndefined();
    expect(
      toElectronCookie(row({ name: '__Host-t', hostKey: '.example.test' }), 'v', NOW_S)
    ).toEqual({ ok: false, reason: 'invalid' });
    const securePrefixed = toElectronCookie(row({ name: '__Secure-s' }), 'v', NOW_S);
    expect(securePrefixed.ok && securePrefixed.details.secure).toBe(true);
  });

  it('maps SameSite, keeps session cookies session cookies, and skips the expired', () => {
    const sameSite = [-1, 0, 1, 2].map((value) => {
      const converted = toElectronCookie(row({ sameSite: value, secure: true }), 'v', NOW_S);
      return converted.ok ? converted.details.sameSite : null;
    });
    expect(sameSite).toEqual(['unspecified', 'no_restriction', 'lax', 'strict']);
    const session = toElectronCookie(row({ hasExpires: false, expiresSeconds: 0 }), 'v', NOW_S);
    expect(session.ok && session.details.expirationDate).toBeUndefined();
    expect(
      toElectronCookie(row({ expiresSeconds: NOW_S + CHROME_EPOCH_OFFSET_SECONDS - 1 }), 'v', NOW_S)
    ).toEqual({ ok: false, reason: 'expired' });
  });

  it('skips partitioned cookies and malformed names, values and hosts', () => {
    expect(toElectronCookie(row({ partitioned: true }), 'v', NOW_S)).toEqual({
      ok: false,
      reason: 'partitioned',
    });
    for (const bad of [
      row({ name: 'has space' }),
      row({ name: 'semi;colon' }),
      row({ hostKey: 'bad host' }),
      row({ hostKey: '' }),
      row({ hostKey: 'evil.test/path' }),
    ]) {
      expect(toElectronCookie(bad, 'v', NOW_S)).toEqual({ ok: false, reason: 'invalid' });
    }
    expect(toElectronCookie(row({}), 'a;b', NOW_S)).toEqual({ ok: false, reason: 'invalid' });
    expect(toElectronCookie(row({}), 'line\nbreak', NOW_S)).toEqual({
      ok: false,
      reason: 'invalid',
    });
  });

  it('groups hosts into sites for the per-site choice', () => {
    expect(cookieSite('.google.com')).toBe('google.com');
    expect(cookieSite('accounts.google.com')).toBe('google.com');
    expect(cookieSite('www.ox.ac.uk')).toBe('ox.ac.uk');
    expect(cookieSite('a.b.co.jp')).toBe('b.co.jp');
    expect(cookieSite('localhost')).toBe('localhost');
    expect(cookieSite('10.0.0.5')).toBe('10.0.0.5');
    expect(
      cookieSiteCounts([
        row({ hostKey: '.google.com' }),
        row({ hostKey: 'mail.google.com' }),
        row({ hostKey: 'ucsf.edu' }),
        row({ hostKey: 'bad host' }),
      ])
    ).toEqual([
      { site: 'google.com', count: 2 },
      { site: 'ucsf.edu', count: 1 },
    ]);
  });
});

describe('Chrome profiles (BB-21)', () => {
  it('finds the data folder for each system', () => {
    expect(chromeDataRoot('darwin', '/Users/p', {})).toBe(
      '/Users/p/Library/Application Support/Google/Chrome'
    );
    expect(chromeDataRoot('linux', '/home/p', {})).toBe('/home/p/.config/google-chrome');
    expect(chromeDataRoot('linux', '/home/p', { XDG_CONFIG_HOME: '/x' })).toBe('/x/google-chrome');
    expect(chromeDataRoot('linux', '/home/p', { XDG_CONFIG_HOME: 'relative' })).toBe(
      '/home/p/.config/google-chrome'
    );
    expect(chromeDataRoot('win32', 'C:\\Users\\p', { LOCALAPPDATA: 'C:\\L' })).toBe(
      path.join('C:\\L', 'Google', 'Chrome', 'User Data')
    );
    expect(chromeDataRoot('win32', 'C:\\Users\\p', {})).toBeNull();
    expect(chromeDataRoot('freebsd', '/home/p', {})).toBeNull();
  });

  it('lists the profiles that exist, the last used first, and nothing else', () => {
    const fixture = buildChromeUserData(path.join(dir, 'Chrome'), { now: NOW_MS });
    const profiles = readChromeProfiles(fixture.root);
    expect(profiles).toEqual([
      { id: 'Default', name: 'Work', email: 'fixture.person@example.test', lastUsed: true },
      { id: 'Profile 1', name: 'Personal', email: null, lastUsed: false },
    ]);
    expect(chromeProfileDir(fixture.root, 'Default')).toBe(path.join(fixture.root, 'Default'));
    // Listed in Local State but not on disk, or not a profile folder name at all.
    for (const id of ['Profile 9', '../escape', '..', 'Default/../..', 'System Profile', '']) {
      expect(chromeProfileDir(fixture.root, id)).toBeNull();
    }
    expect(chromeCookiesFile(path.join(fixture.root, 'Default'))).toBe(
      path.join(fixture.root, 'Default', 'Network', 'Cookies')
    );
    expect(chromeCookiesFile(path.join(fixture.root, 'Profile 1'))).toBe(
      path.join(fixture.root, 'Profile 1', 'Cookies')
    );
    // The import's file list never names saved passwords or payment data.
    const files = Object.values(chromeProfileFiles(path.join(fixture.root, 'Default')));
    expect(files.some((file) => /Login Data|Web Data/.test(String(file)))).toBe(false);
  });

  it('treats a folder with no Local State, or none at all, sensibly', () => {
    expect(readChromeProfiles(path.join(dir, 'missing'))).toEqual([]);
    fs.mkdirSync(path.join(dir, 'Bare', 'Default'), { recursive: true });
    expect(readChromeProfiles(path.join(dir, 'Bare'))).toEqual([
      { id: 'Default', name: 'Default', email: null, lastUsed: true },
    ]);
    fs.writeFileSync(path.join(dir, 'Bare', 'Local State'), '{ not json');
    expect(readChromeProfiles(path.join(dir, 'Bare'))).toHaveLength(1);
  });
});

describe('Chrome bookmarks and preferences', () => {
  it('reads bookmarks with their folders and skips what is not a web page', () => {
    const fixture = buildChromeUserData(path.join(dir, 'Chrome'), { now: NOW_MS });
    const bookmarks = readChromeBookmarks(path.join(fixture.root, 'Default', 'Bookmarks'));
    expect(bookmarks.map((bookmark) => [bookmark.folder.join(' / '), bookmark.title])).toEqual([
      ['Bookmarks bar', 'PubMed'],
      ['Bookmarks bar', 'Fixture dashboard'],
      ['Bookmarks bar / Research', 'ClinicalTrials.gov'],
      ['Bookmarks bar / Research / Genes', 'NCBI Gene'],
      ['Other bookmarks', 'UCSF'],
    ]);
    expect(bookmarks[0].addedAt).toBe(NOW_MS - 5 * 24 * 60 * 60 * 1000);
    expect(readChromeBookmarks(path.join(dir, 'missing'))).toEqual([]);
  });

  it('converts Chrome time stamps', () => {
    expect(chromeMicrosToUnixMs('13300000000000000')).toBe(13300000000000 - 11644473600000);
    expect(chromeMicrosToUnixMs(0)).toBe(0);
    expect(chromeMicrosToUnixMs('not a number')).toBe(0);
    expect(chromeMicrosToUnixMs(-5)).toBe(0);
  });

  it('takes the search engine, the languages and the default zoom', () => {
    const fixture = buildChromeUserData(path.join(dir, 'Chrome'), { now: NOW_MS });
    expect(readChromePreferences(path.join(fixture.root, 'Default', 'Preferences'))).toEqual({
      searchEngine: { name: 'DuckDuckGo', template: 'https://duckduckgo.com/?q={searchTerms}' },
      acceptLanguages: 'en-US,en,fr-CA',
      defaultZoomFactor: 1.25,
    });
    const file = path.join(dir, 'Preferences');
    fs.writeFileSync(
      file,
      JSON.stringify({
        default_search_provider_data: {
          template_url_data: { short_name: 'Plain', url: 'http://search.test/?q={searchTerms}' },
        },
        partition: { default_zoom_level: { x: 0 } },
      })
    );
    expect(readChromePreferences(file)).toEqual({
      searchEngine: null,
      acceptLanguages: null,
      defaultZoomFactor: null,
    });
  });
});
