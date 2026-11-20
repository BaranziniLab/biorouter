// Synthetic Chrome profiles for the Biorouter Browser's Chrome import
// (docs/desktop-ui/built-in-browser.md, "Synthetic Chrome profiles").
//
// Writes, into a folder of the caller's choosing, a Chrome user data folder that follows Chrome's
// real on-disk formats: `Local State`, a cookie database at the schema of Chromium's
// `sqlite_persistent_cookie_store.cc`, `Bookmarks`, `History` and `Preferences`. Cookie values are
// encrypted here, independently of the importer's own code, the way Chrome encrypts them:
// PBKDF2-HMAC-SHA1 over a password with the salt `saltysalt`, then AES-128-CBC with an IV of 16
// spaces, behind a `v10` or `v11` prefix, and from database version 24 behind the SHA-256 of the
// cookie's host.
//
// ⚠ Nothing here is copied from, or reads, a real Chrome profile, a real Keychain or a real key.
// The password below is a test value and exists only in test files.
//
// By hand, for a live check in the dev app:
//   node tests/fixtures/chrome-profile/build.mjs <empty folder> [darwin|linux] [--json]
// prints the user data folder to point BIOROUTER_BROWSER_TEST_CHROME_DIR at, or with --json the
// whole description (the Playwright spec runs it this way: its transform cannot load ESM).

import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { pathToFileURL } from 'node:url';

export const FIXTURE_PASSWORD = 'fixture-safe-storage-password';
const EPOCH_OFFSET_MS = 11_644_473_600_000n;
const IV = Buffer.alloc(16, 0x20);
const DAY_MS = 24 * 60 * 60 * 1000;

function key(password, iterations) {
  return crypto.pbkdf2Sync(password, 'saltysalt', iterations, 16, 'sha1');
}

function encrypt(value, aesKey, version, hostKey, digest) {
  const body = Buffer.from(value, 'utf8');
  const plaintext = digest
    ? Buffer.concat([crypto.createHash('sha256').update(hostKey).digest(), body])
    : body;
  const cipher = crypto.createCipheriv('aes-128-cbc', aesKey, IV);
  return Buffer.concat([Buffer.from(version), cipher.update(plaintext), cipher.final()]);
}

/** Chrome's microseconds since 1601 for a time in milliseconds since 1970. */
export function chromeMicros(unixMs) {
  return (BigInt(Math.trunc(unixMs)) + EPOCH_OFFSET_MS) * 1000n;
}

const COOKIE_SCHEMA_24 = `
CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);
CREATE TABLE cookies(creation_utc INTEGER NOT NULL,host_key TEXT NOT NULL,top_frame_site_key TEXT NOT NULL,
  name TEXT NOT NULL,value TEXT NOT NULL,encrypted_value BLOB NOT NULL,path TEXT NOT NULL,
  expires_utc INTEGER NOT NULL,is_secure INTEGER NOT NULL,is_httponly INTEGER NOT NULL,
  last_access_utc INTEGER NOT NULL,has_expires INTEGER NOT NULL,is_persistent INTEGER NOT NULL,
  priority INTEGER NOT NULL,samesite INTEGER NOT NULL,source_scheme INTEGER NOT NULL,
  source_port INTEGER NOT NULL,last_update_utc INTEGER NOT NULL,source_type INTEGER NOT NULL,
  has_cross_site_ancestor INTEGER NOT NULL);
CREATE UNIQUE INDEX cookies_unique_index ON cookies(host_key, top_frame_site_key,
  has_cross_site_ancestor, name, path, source_scheme, source_port);`;

// Version 23 and earlier: no `source_type`, no `has_cross_site_ancestor`.
const COOKIE_SCHEMA_23 = `
CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);
CREATE TABLE cookies(creation_utc INTEGER NOT NULL,host_key TEXT NOT NULL,top_frame_site_key TEXT NOT NULL,
  name TEXT NOT NULL,value TEXT NOT NULL,encrypted_value BLOB NOT NULL,path TEXT NOT NULL,
  expires_utc INTEGER NOT NULL,is_secure INTEGER NOT NULL,is_httponly INTEGER NOT NULL,
  last_access_utc INTEGER NOT NULL,has_expires INTEGER NOT NULL,is_persistent INTEGER NOT NULL,
  priority INTEGER NOT NULL,samesite INTEGER NOT NULL,source_scheme INTEGER NOT NULL,
  source_port INTEGER NOT NULL,last_update_utc INTEGER NOT NULL);`;

/**
 * Write a cookie database. Each cookie: { host, name, value, path?, secure?, httpOnly?, sameSite?,
 * expiresMs? (null for a session cookie), topFrameSiteKey?, version? ('v10'|'v11'|raw prefix),
 * encrypted? (Buffer, overrides), plain? (store in the plaintext column) }.
 */
function writeCookies(file, { schemaVersion, keys, cookies, now }) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const db = new DatabaseSync(file);
  db.exec(schemaVersion >= 24 ? COOKIE_SCHEMA_24 : COOKIE_SCHEMA_23);
  db.prepare('INSERT INTO meta(key, value) VALUES (?, ?)').run('version', String(schemaVersion));
  db.prepare('INSERT INTO meta(key, value) VALUES (?, ?)').run(
    'last_compatible_version',
    String(Math.min(schemaVersion, 24))
  );
  const columns =
    'creation_utc,host_key,top_frame_site_key,name,value,encrypted_value,path,expires_utc,is_secure,is_httponly,last_access_utc,has_expires,is_persistent,priority,samesite,source_scheme,source_port,last_update_utc' +
    (schemaVersion >= 24 ? ',source_type,has_cross_site_ancestor' : '');
  const count = columns.split(',').length;
  const insert = db.prepare(
    `INSERT INTO cookies(${columns}) VALUES (${new Array(count).fill('?').join(',')})`
  );
  const created = chromeMicros(now - 10 * DAY_MS);
  for (const cookie of cookies) {
    const version = cookie.version ?? 'v10';
    const encrypted = cookie.plain
      ? Buffer.alloc(0)
      : (cookie.encrypted ??
        encrypt(cookie.value, keys[version], version, cookie.host, schemaVersion >= 24));
    const persistent = cookie.expiresMs !== null;
    const values = [
      created,
      cookie.host,
      cookie.topFrameSiteKey ?? '',
      cookie.name,
      cookie.plain ? cookie.value : '',
      encrypted,
      cookie.path ?? '/',
      persistent ? chromeMicros(cookie.expiresMs ?? now + 30 * DAY_MS) : 0n,
      cookie.secure ? 1 : 0,
      cookie.httpOnly ? 1 : 0,
      created,
      persistent ? 1 : 0,
      persistent ? 1 : 0,
      1,
      cookie.sameSite ?? -1,
      cookie.secure ? 2 : 1,
      cookie.secure ? 443 : 80,
      created,
    ];
    if (schemaVersion >= 24) values.push(0, 0);
    insert.run(...values);
  }
  db.close();
}

function writeHistory(file, now) {
  const db = new DatabaseSync(file);
  db.exec(`
    CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);
    CREATE TABLE urls(id INTEGER PRIMARY KEY AUTOINCREMENT,url LONGVARCHAR,title LONGVARCHAR,
      visit_count INTEGER DEFAULT 0 NOT NULL,typed_count INTEGER DEFAULT 0 NOT NULL,
      last_visit_time INTEGER NOT NULL,hidden INTEGER DEFAULT 0 NOT NULL);
    CREATE TABLE visits(id INTEGER PRIMARY KEY,url INTEGER NOT NULL,visit_time INTEGER NOT NULL,
      from_visit INTEGER,transition INTEGER DEFAULT 0 NOT NULL,segment_id INTEGER,
      visit_duration INTEGER DEFAULT 0 NOT NULL);`);
  db.prepare("INSERT INTO meta VALUES('version','67')").run();
  const insertUrl = db.prepare(
    'INSERT INTO urls(url,title,visit_count,typed_count,last_visit_time,hidden) VALUES (?,?,?,?,?,?)'
  );
  const insertVisit = db.prepare('INSERT INTO visits(url,visit_time,transition) VALUES (?,?,0)');
  // One page a day for 120 days, newest first: 90 are inside the import window.
  for (let day = 0; day < 120; day += 1) {
    const when = now - day * DAY_MS - 60_000;
    const result = insertUrl.run(
      `https://pubmed.ncbi.nlm.nih.gov/fixture-${day}/`,
      `Fixture article ${day}`,
      3,
      day % 7 === 0 ? 1 : 0,
      chromeMicros(when),
      0
    );
    insertVisit.run(result.lastInsertRowid, chromeMicros(when));
  }
  insertUrl.run('https://site-a.test/dashboard', 'Fixture dashboard', 9, 4, chromeMicros(now - DAY_MS), 0);
  insertUrl.run('https://hidden.test/', 'Hidden row', 1, 0, chromeMicros(now - DAY_MS), 1);
  insertUrl.run('chrome://settings/', 'Settings', 1, 0, chromeMicros(now - DAY_MS), 0);
  insertUrl.run('file:///Users/someone/notes.txt', 'Local file', 1, 0, chromeMicros(now - DAY_MS), 0);
  db.close();
}

function writeBookmarks(file, now) {
  const added = (days) => String(chromeMicros(now - days * DAY_MS));
  let id = 0;
  const url = (name, href, days = 5) => ({
    date_added: added(days),
    guid: crypto.randomUUID(),
    id: String((id += 1)),
    name,
    type: 'url',
    url: href,
  });
  const folder = (name, children) => ({
    children,
    date_added: added(30),
    date_modified: added(1),
    guid: crypto.randomUUID(),
    id: String((id += 1)),
    name,
    type: 'folder',
  });
  const bookmarks = {
    checksum: '00000000000000000000000000000000',
    roots: {
      bookmark_bar: folder('Bookmarks bar', [
        url('PubMed', 'https://pubmed.ncbi.nlm.nih.gov/'),
        url('Fixture dashboard', 'https://site-a.test/dashboard'),
        folder('Research', [
          url('ClinicalTrials.gov', 'https://clinicaltrials.gov/'),
          folder('Genes', [url('NCBI Gene', 'https://www.ncbi.nlm.nih.gov/gene')]),
          url('A script', 'javascript:alert(1)'),
        ]),
      ]),
      other: folder('Other bookmarks', [url('UCSF', 'https://www.ucsf.edu/')]),
      synced: folder('Mobile bookmarks', []),
    },
    version: 1,
  };
  fs.writeFileSync(file, JSON.stringify(bookmarks, null, 3));
}

function writePreferences(file) {
  const preferences = {
    default_search_provider_data: {
      template_url_data: {
        short_name: 'DuckDuckGo',
        keyword: 'duckduckgo.com',
        url: 'https://duckduckgo.com/?q={searchTerms}',
      },
    },
    intl: { accept_languages: 'en-US,en,fr-CA,not a language' },
    partition: { default_zoom_level: { x: 1.2239010857415449 } },
    profile: { name: 'Work' },
  };
  fs.writeFileSync(file, JSON.stringify(preferences));
}

/** The sentinel files the importer must never open: saved passwords and payment data. */
export const SENTINELS = ['Login Data', 'Web Data'];

function writeSentinels(profileDir) {
  for (const name of SENTINELS) {
    fs.writeFileSync(path.join(profileDir, name), 'sentinel: the importer must never open this');
    fs.chmodSync(path.join(profileDir, name), 0o000);
  }
}

/**
 * Build a Chrome user data folder at `root`. Returns what the tests assert against: the
 * password, every cookie value the database holds (for the logger spy), and the expected
 * outcome of each cookie.
 */
export function buildChromeUserData(root, options = {}) {
  const platform = options.platform ?? 'darwin';
  const now = options.now ?? Date.now();
  const password = options.password ?? FIXTURE_PASSWORD;
  fs.mkdirSync(root, { recursive: true });

  const keys =
    platform === 'darwin'
      ? { v10: key(password, 1003) }
      : { v10: key('peanuts', 1), v11: key(password, 1) };
  const future = now + 30 * DAY_MS;

  fs.writeFileSync(
    path.join(root, 'Local State'),
    JSON.stringify({
      profile: {
        info_cache: {
          Default: {
            name: 'Work',
            user_name: 'fixture.person@example.test',
            gaia_name: 'Fixture Person',
          },
          'Profile 1': { name: 'Personal', user_name: '' },
          'Profile 9': { name: 'Listed but missing on disk' },
          '../escape': { name: 'Not a profile folder' },
        },
        last_used: 'Default',
      },
    })
  );

  const defaultDir = path.join(root, 'Default');
  fs.mkdirSync(defaultDir, { recursive: true });
  // Every row, its value and what the importer should do with it.
  const cookies = [
    { host: 'site-a.test', name: 'fixture_session', value: 'fixture-user', httpOnly: true, sameSite: 1, expect: 'imported' },
    { host: '.site-a.test', name: 'prefs', value: 'fxv-domain-theme-dark', sameSite: 1, expect: 'imported' },
    { host: 'site-a.test', name: '__Host-token', value: 'fxv-host-prefix-token', secure: true, sameSite: 2, expect: 'imported' },
    { host: '.site-b.test', name: '__Secure-id', value: 'fxv-secure-prefix-id', secure: true, sameSite: 0, expect: 'imported' },
    { host: 'site-b.test', name: 'session_only', value: 'fxv-session-cookie', expiresMs: null, expect: 'imported' },
    { host: 'site-b.test', name: 'lax_cookie', value: 'fxv-lax', sameSite: 1, expect: 'imported' },
    { host: 'site-b.test', name: 'strict_cookie', value: 'fxv-strict', sameSite: 2, expect: 'imported' },
    { host: 'site-b.test', name: 'unspecified_cookie', value: 'fxv-unspecified', sameSite: -1, expect: 'imported' },
    { host: 'www.ox.ac.uk', name: 'uni', value: 'fxv-second-level', expect: 'imported' },
    { host: 'site-a.test', name: 'plain_column', value: 'fxv-plaintext-column', plain: true, expect: 'imported' },
    { host: 'site-a.test', name: 'old_cookie', value: 'fxv-expired', expiresMs: now - DAY_MS, expect: 'expired' },
    { host: 'site-b.test', name: 'partitioned', value: 'fxv-partitioned', secure: true, topFrameSiteKey: 'https://site-a.test', expect: 'partitioned' },
    { host: 'site-a.test', name: 'wrong_digest', value: 'fxv-wrong-digest', encryptedFor: 'site-b.test', expect: 'digest-mismatch' },
    { host: 'site-a.test', name: 'unknown_prefix', value: 'fxv-unknown-prefix', rawPrefix: 'v99', expect: 'unknown-prefix' },
    { host: 'site-a.test', name: 'app_bound', value: 'fxv-app-bound', rawPrefix: 'v20', expect: 'app-bound' },
    { host: 'site-a.test', name: 'bad_padding', value: 'fxv-bad-padding', truncate: true, expect: 'bad-ciphertext' },
    { host: 'bad host!', name: 'bad_host', value: 'fxv-bad-host', expect: 'invalid' },
    { host: '.site-a.test', name: '__Host-domain', value: 'fxv-host-with-domain', secure: true, expect: 'invalid' },
  ].map((cookie) => {
    const out = { ...cookie, expiresMs: cookie.expiresMs === undefined ? future : cookie.expiresMs };
    if (cookie.encryptedFor) {
      out.encrypted = encrypt(cookie.value, keys.v10, 'v10', cookie.encryptedFor, true);
    } else if (cookie.rawPrefix) {
      const body = encrypt(cookie.value, keys.v10, 'v10', cookie.host, true);
      out.encrypted = Buffer.concat([Buffer.from(cookie.rawPrefix), body.subarray(3)]);
    } else if (cookie.truncate) {
      const body = encrypt(cookie.value, keys.v10, 'v10', cookie.host, true);
      out.encrypted = body.subarray(0, body.length - 5);
    }
    if (platform === 'linux' && !cookie.plain && !out.encrypted) {
      // Linux holds both kinds: the fixed password and the key store's.
      out.version = cookie.name.length % 2 === 0 ? 'v11' : 'v10';
    }
    return out;
  });
  writeCookies(path.join(defaultDir, 'Network', 'Cookies'), {
    schemaVersion: 24,
    keys,
    cookies,
    now,
  });
  writeBookmarks(path.join(defaultDir, 'Bookmarks'), now);
  writeHistory(path.join(defaultDir, 'History'), now);
  writePreferences(path.join(defaultDir, 'Preferences'));
  writeSentinels(defaultDir);

  // An older profile: `Cookies` at the old location, version 23, no digest prefix.
  const personalDir = path.join(root, 'Profile 1');
  fs.mkdirSync(personalDir, { recursive: true });
  const personalCookies = [
    { host: '.site-b.test', name: 'personal', value: 'fxv-personal-v23', expiresMs: future, expect: 'imported' },
  ];
  writeCookies(path.join(personalDir, 'Cookies'), {
    schemaVersion: 23,
    keys,
    cookies: personalCookies,
    now,
  });
  writeSentinels(personalDir);

  return {
    root,
    platform,
    password,
    now,
    cookies,
    personalCookies,
    /** Every secret the fixture holds, for tests that must never see one in a log or a file. */
    secrets: [
      password,
      ...Object.values(keys).flatMap((aesKey) => [aesKey.toString('hex'), aesKey.toString('base64')]),
      ...[...cookies, ...personalCookies].map((cookie) => cookie.value),
    ],
  };
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const target = process.argv[2];
  if (!target) {
    console.error('usage: node build.mjs <empty folder> [darwin|linux]');
    process.exit(2);
  }
  const built = buildChromeUserData(path.resolve(target), {
    platform: process.argv.includes('linux') ? 'linux' : 'darwin',
  });
  if (process.argv.includes('--json')) {
    const { cookies, personalCookies, ...rest } = built;
    const plain = (list) => list.map(({ encrypted: _encrypted, ...cookie }) => cookie);
    console.log(
      JSON.stringify({ ...rest, cookies: plain(cookies), personalCookies: plain(personalCookies) })
    );
  } else {
    console.log(built.root);
  }
}
