import type { DatabaseSync } from 'node:sqlite';
import { metaVersion, tableColumns } from './sqliteCopy';

/**
 * Chrome's cookie table, read and converted for `session.cookies.set` (BB-22).
 *
 * Column names follow Chromium's `sqlite_persistent_cookie_store.cc`. Older schemas named the
 * flags `secure` and `httponly`, and lacked `samesite` and `top_frame_site_key`; the reader asks
 * the table which columns it has rather than assuming one version.
 *
 * Times are microseconds since 1601-01-01 UTC, which overflow a JavaScript number's exact range,
 * so the query divides them to whole seconds in SQLite.
 */

/** Seconds between 1601-01-01 and 1970-01-01. */
export const CHROME_EPOCH_OFFSET_SECONDS = 11_644_473_600;
/** A profile with more cookies than this is read up to the limit. */
export const MAX_COOKIE_ROWS = 50_000;

export type ChromeCookieRow = {
  hostKey: string;
  name: string;
  /** Chrome's plaintext column, used by very old profiles and when encryption was unavailable. */
  plainValue: string;
  encryptedValue: Uint8Array;
  path: string;
  /** Seconds since 1601, 0 for none. */
  expiresSeconds: number;
  hasExpires: boolean;
  secure: boolean;
  httpOnly: boolean;
  /** -1 unspecified, 0 none, 1 lax, 2 strict. */
  sameSite: number;
  partitioned: boolean;
};

export type CookieDatabase = { version: number; rows: ChromeCookieRow[]; truncated: boolean };

function column(columns: Set<string>, ...names: string[]): string | null {
  return names.find((name) => columns.has(name)) ?? null;
}

export function readCookieDatabase(db: DatabaseSync): CookieDatabase {
  const columns = tableColumns(db, 'cookies');
  if (!columns.has('host_key') || !columns.has('name')) {
    return { version: metaVersion(db), rows: [], truncated: false };
  }
  const pick = (alias: string, fallback: string, ...names: string[]) => {
    const name = column(columns, ...names);
    return `${name ?? fallback} AS ${alias}`;
  };
  const sql = [
    'SELECT host_key AS host_key, name AS name',
    pick('value', "''", 'value'),
    pick('encrypted_value', "X''", 'encrypted_value'),
    pick('path', "'/'", 'path'),
    `${column(columns, 'expires_utc') ? 'expires_utc / 1000000' : '0'} AS expires_seconds`,
    pick('has_expires', '1', 'has_expires'),
    pick('secure', '0', 'is_secure', 'secure'),
    pick('httponly', '0', 'is_httponly', 'httponly'),
    pick('samesite', '-1', 'samesite'),
    pick('top_frame_site_key', "''", 'top_frame_site_key'),
  ].join(', ');
  const statement = db.prepare(
    `${sql} FROM cookies ORDER BY host_key, name LIMIT ${MAX_COOKIE_ROWS + 1}`
  );
  const raw = statement.all() as Array<Record<string, unknown>>;
  const truncated = raw.length > MAX_COOKIE_ROWS;
  const rows: ChromeCookieRow[] = [];
  for (const row of raw.slice(0, MAX_COOKIE_ROWS)) {
    const encrypted = row.encrypted_value;
    rows.push({
      hostKey: String(row.host_key ?? ''),
      name: String(row.name ?? ''),
      plainValue: typeof row.value === 'string' ? row.value : '',
      encryptedValue:
        encrypted instanceof Uint8Array
          ? encrypted
          : typeof encrypted === 'string'
            ? Buffer.from(encrypted, 'latin1')
            : new Uint8Array(),
      path: String(row.path ?? '/') || '/',
      expiresSeconds: Number(row.expires_seconds) || 0,
      hasExpires: Number(row.has_expires) !== 0,
      secure: Number(row.secure) !== 0,
      httpOnly: Number(row.httponly) !== 0,
      sameSite: Number.isFinite(Number(row.samesite)) ? Number(row.samesite) : -1,
      partitioned: String(row.top_frame_site_key ?? '') !== '',
    });
  }
  return { version: metaVersion(db), rows, truncated };
}

const HOST = /^\.?(?:[a-z0-9_](?:[a-z0-9_-]*[a-z0-9_])?\.)*[a-z0-9_](?:[a-z0-9_-]*[a-z0-9_])?$/i;
const IPV4 = /^\d{1,3}(?:\.\d{1,3}){3}$/;

export function isCookieHost(hostKey: string): boolean {
  return hostKey.length > 0 && hostKey.length <= 253 && HOST.test(hostKey);
}

/** Second level labels under which sites register their own names (`ox.ac.uk`, `abc.co.jp`). */
const SHARED_SECOND_LEVEL = new Set([
  'ac',
  'co',
  'com',
  'edu',
  'gov',
  'gv',
  'ltd',
  'me',
  'mil',
  'ne',
  'net',
  'nhs',
  'or',
  'org',
  'plc',
  'sch',
]);

/**
 * The site a cookie belongs to, for the per-site choice: its registrable name, approximated from
 * the host without the public suffix list. `accounts.google.com` and `.google.com` are one row
 * (`google.com`); `www.ox.ac.uk` is `ox.ac.uk`; an address stays itself.
 */
export function cookieSite(hostKey: string): string {
  const host = hostKey.replace(/^\./, '').toLowerCase();
  if (IPV4.test(host)) return host;
  const labels = host.split('.');
  if (labels.length <= 2) return host;
  const secondLevel = labels[labels.length - 2];
  const topLevel = labels[labels.length - 1];
  const keep = SHARED_SECOND_LEVEL.has(secondLevel) && topLevel.length === 2 ? 3 : 2;
  return labels.slice(-keep).join('.');
}

export type CookieSiteCount = { site: string; count: number };

/** Sites and how many cookies each holds, from the unencrypted `host_key` column. */
export function cookieSiteCounts(rows: ChromeCookieRow[]): CookieSiteCount[] {
  const counts = new Map<string, number>();
  for (const row of rows) {
    if (!isCookieHost(row.hostKey)) continue;
    const site = cookieSite(row.hostKey);
    counts.set(site, (counts.get(site) ?? 0) + 1);
  }
  return [...counts]
    .map(([site, count]) => ({ site, count }))
    .sort((a, b) => b.count - a.count || a.site.localeCompare(b.site));
}

/** What `session.cookies.set` takes, less nothing. */
export type ElectronCookieDetails = {
  url: string;
  name: string;
  value: string;
  domain?: string;
  path: string;
  secure: boolean;
  httpOnly: boolean;
  expirationDate?: number;
  sameSite: 'unspecified' | 'no_restriction' | 'lax' | 'strict';
};

/** What the import records about each cookie it set, so removal can find it. Never a value. */
export type ImportedCookieKey = { url: string; name: string; domain?: string; path: string };

export type CookieSkipReason = 'expired' | 'partitioned' | 'invalid';

const SAME_SITE: Record<number, ElectronCookieDetails['sameSite']> = {
  [-1]: 'unspecified',
  0: 'no_restriction',
  1: 'lax',
  2: 'strict',
};

// RFC 6265 token characters for the name; the value may hold anything but controls, `;` and `,`
// is tolerated as browsers do. Electron refuses the rest, so they are counted as invalid here.
const COOKIE_NAME = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]*$/;
// eslint-disable-next-line no-control-regex
const COOKIE_VALUE_FORBIDDEN = /[\u0000-\u001f\u007f;]/;

export function toElectronCookie(
  row: ChromeCookieRow,
  value: string,
  nowSeconds: number
):
  | { ok: true; details: ElectronCookieDetails; key: ImportedCookieKey }
  | { ok: false; reason: CookieSkipReason } {
  if (row.partitioned) return { ok: false, reason: 'partitioned' };
  if (!isCookieHost(row.hostKey) || !COOKIE_NAME.test(row.name)) {
    return { ok: false, reason: 'invalid' };
  }
  if (COOKIE_VALUE_FORBIDDEN.test(value) || value.length > 4096) {
    return { ok: false, reason: 'invalid' };
  }

  let expirationDate: number | undefined;
  if (row.hasExpires) {
    expirationDate = row.expiresSeconds - CHROME_EPOCH_OFFSET_SECONDS;
    if (!(expirationDate > nowSeconds)) return { ok: false, reason: 'expired' };
  }

  const isDomainCookie = row.hostKey.startsWith('.');
  const host = row.hostKey.replace(/^\./, '').toLowerCase();
  let path = row.path.startsWith('/') ? row.path : '/';
  let secure = row.secure;
  let domain: string | undefined = isDomainCookie ? `.${host}` : undefined;

  if (row.name.startsWith('__Host-')) {
    // A `__Host-` cookie is secure, host only and for the whole host by definition. One stored
    // with a domain is not a valid `__Host-` cookie at all.
    if (isDomainCookie) return { ok: false, reason: 'invalid' };
    secure = true;
    path = '/';
    domain = undefined;
  } else if (row.name.startsWith('__Secure-')) {
    secure = true;
  }

  const details: ElectronCookieDetails = {
    url: `${secure ? 'https' : 'http'}://${host}${path}`,
    name: row.name,
    value,
    path,
    secure,
    httpOnly: row.httpOnly,
    sameSite: SAME_SITE[row.sameSite] ?? 'unspecified',
  };
  // A host only cookie gets no `domain`: Electron would widen it to every subdomain.
  if (domain) details.domain = domain;
  if (expirationDate !== undefined) details.expirationDate = expirationDate;
  const key: ImportedCookieKey = { url: details.url, name: row.name, path };
  if (domain) key.domain = domain;
  return { ok: true, details, key };
}
