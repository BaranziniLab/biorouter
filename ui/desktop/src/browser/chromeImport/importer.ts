import fs from 'node:fs';
import os from 'node:os';
import {
  cookieValueVersion,
  decryptCookieValue,
  DIGEST_PREFIX_VERSION,
  type CookieKeyVersion,
  type CookieKeys,
} from './cookieCrypto';
import {
  cookieSite,
  cookieSiteCounts,
  readCookieDatabase,
  toElectronCookie,
  type ChromeCookieRow,
  type CookieSiteCount,
  type ElectronCookieDetails,
  type ImportedCookieKey,
} from './cookies';
import type { KeyFailure, KeyProvider } from './keyProvider';
import {
  readChromeBookmarks,
  readChromeHistory,
  readChromePreferences,
  type ChromeBookmark,
  type ChromeHistoryEntry,
  type ChromePreferences,
} from './profileData';
import { chromeProfileFiles } from './profiles';
import { UnreadableDatabaseError, withCopiedDatabase } from './sqliteCopy';

/**
 * One Chrome import, from a profile folder into the Biorouter Browser (BB-21 to BB-25).
 *
 * Pure apart from reading the profile: every write goes through `targets`, so the same code runs
 * against Electron's cookie jar and the browser store in the app, and against fakes in the tests.
 *
 * ⚠ **Values never leave this function except into `targets.cookies.set`.** The report holds
 * counts, site names and the keys of the cookies set (url, name, domain, path), never a value; the
 * log gets counts and sites. Keys are overwritten when the import ends.
 */

export type ImportChoices = {
  logins: boolean;
  bookmarks: boolean;
  history: boolean;
  settings: boolean;
};

export type CookieJar = { set(details: ElectronCookieDetails): Promise<void> };

export type AppliedSettings = Array<'search engine' | 'languages' | 'default zoom'>;

export type ImportTargets = {
  cookies: CookieJar;
  addBookmarks(items: ChromeBookmark[]): number;
  addHistory(items: ChromeHistoryEntry[]): number;
  applySettings(preferences: ChromePreferences): AppliedSettings;
};

export type ImportLog = (message: string, details?: Record<string, unknown>) => void;

export type LoginsOutcome =
  | 'imported'
  | 'not-chosen'
  | 'none-found'
  | 'unreadable'
  | 'unsupported'
  | KeyFailure;

export type ImportReport = {
  logins: LoginsOutcome;
  /** Logins imported, but one key version could not be read, so its cookies were skipped (L5). */
  loginsMissingKey: KeyFailure | null;
  cookies: {
    imported: number;
    sites: number;
    /** Cookies left out on purpose: expired, partitioned, locked to Chrome, malformed. */
    skipped: Record<string, number>;
    /** Cookies the browser refused to take, by site. */
    failedBySite: Record<string, number>;
    truncated: boolean;
  };
  /** `null` when the person did not choose it, `-1` when the file could not be read. */
  bookmarks: number | null;
  history: number | null;
  settings: AppliedSettings | null;
  /** Recorded in the import manifest so **Remove imported data** can find each cookie. */
  cookieKeys: ImportedCookieKey[];
};

export type CookieSitePlan =
  | { readable: true; sites: CookieSiteCount[]; total: number }
  | { readable: false; sites: []; total: 0 };

const SET_BATCH = 64;

/**
 * The sites with cookies in a profile, before any key is requested: `host_key` is not encrypted.
 */
export function planCookieSites(profileDir: string, tempRoot = os.tmpdir()): CookieSitePlan {
  const file = chromeProfileFiles(profileDir).cookies;
  if (!file) return { readable: true, sites: [], total: 0 };
  try {
    const { rows } = withCopiedDatabase(file, readCookieDatabase, tempRoot);
    const sites = cookieSiteCounts(rows.filter((row) => !row.partitioned));
    return { readable: true, sites, total: sites.reduce((sum, site) => sum + site.count, 0) };
  } catch {
    return { readable: false, sites: [], total: 0 };
  }
}

function count(record: Record<string, number>, key: string, by = 1): void {
  record[key] = (record[key] ?? 0) + by;
}

function wipe(keys: CookieKeys): void {
  for (const key of Object.values(keys)) key?.fill(0);
}

async function importCookies(
  rows: ChromeCookieRow[],
  digestPrefixed: boolean,
  keyProvider: KeyProvider,
  targets: ImportTargets,
  nowSeconds: number,
  report: ImportReport
): Promise<LoginsOutcome> {
  const needed = new Set<CookieKeyVersion>();
  for (const row of rows) {
    const version = row.encryptedValue.length > 0 ? cookieValueVersion(row.encryptedValue) : null;
    if (version === 'v10' || version === 'v11') needed.add(version);
  }

  let keys: CookieKeys = {};
  if (needed.size > 0) {
    const result = await keyProvider(needed);
    if (!result.ok) return result.reason;
    keys = result.keys;
    report.loginsMissingKey = result.missing ?? null;
  }

  try {
    const sites = new Set<string>();
    const pending: Array<{ details: ElectronCookieDetails; key: ImportedCookieKey; site: string }> =
      [];
    const flush = async () => {
      const batch = pending.splice(0, pending.length);
      const results = await Promise.allSettled(
        batch.map((item) => targets.cookies.set(item.details))
      );
      results.forEach((result, index) => {
        const item = batch[index];
        if (result.status === 'fulfilled') {
          report.cookies.imported += 1;
          report.cookieKeys.push(item.key);
          sites.add(item.site);
        } else {
          count(report.cookies.failedBySite, item.site);
        }
        // The value is not needed once the jar has it.
        item.details.value = '';
      });
    };

    for (const row of rows) {
      let value: string;
      if (row.encryptedValue.length > 0) {
        const decrypted = decryptCookieValue(row.encryptedValue, keys, {
          hostKey: row.hostKey,
          digestPrefixed,
        });
        if (!decrypted.ok) {
          count(report.cookies.skipped, decrypted.reason);
          continue;
        }
        value = decrypted.value;
      } else {
        value = row.plainValue;
      }
      const converted = toElectronCookie(row, value, nowSeconds);
      value = '';
      if (!converted.ok) {
        count(report.cookies.skipped, converted.reason);
        continue;
      }
      pending.push({
        details: converted.details,
        key: converted.key,
        site: cookieSite(row.hostKey),
      });
      if (pending.length >= SET_BATCH) await flush();
    }
    await flush();
    report.cookies.sites = sites.size;
    return 'imported';
  } finally {
    wipe(keys);
  }
}

export async function runChromeImport(options: {
  profileDir: string;
  choices: ImportChoices;
  /** The sites to import cookies for, `null` for all. */
  sites: ReadonlySet<string> | null;
  keyProvider: KeyProvider;
  targets: ImportTargets;
  nowMs: number;
  /** False where logins cannot be imported at all (Windows, BB-24). */
  loginsSupported: boolean;
  log: ImportLog;
  tempRoot?: string;
}): Promise<ImportReport> {
  const { profileDir, choices, targets, log } = options;
  const tempRoot = options.tempRoot ?? os.tmpdir();
  const files = chromeProfileFiles(profileDir);
  const report: ImportReport = {
    logins: 'not-chosen',
    loginsMissingKey: null,
    cookies: { imported: 0, sites: 0, skipped: {}, failedBySite: {}, truncated: false },
    bookmarks: null,
    history: null,
    settings: null,
    cookieKeys: [],
  };

  if (choices.logins) {
    if (!options.loginsSupported) {
      report.logins = 'unsupported';
    } else if (!files.cookies) {
      report.logins = 'none-found';
    } else {
      let database: ReturnType<typeof readCookieDatabase> | null = null;
      try {
        database = withCopiedDatabase(files.cookies, readCookieDatabase, tempRoot);
      } catch (error) {
        if (!(error instanceof UnreadableDatabaseError)) throw error;
        report.logins = 'unreadable';
      }
      if (database) {
        report.cookies.truncated = database.truncated;
        const chosen = database.rows.filter(
          (row) => !options.sites || options.sites.has(cookieSite(row.hostKey))
        );
        report.logins =
          chosen.length === 0
            ? 'none-found'
            : await importCookies(
                chosen,
                database.version >= DIGEST_PREFIX_VERSION,
                options.keyProvider,
                targets,
                Math.floor(options.nowMs / 1000),
                report
              );
        database.rows.length = 0;
      }
    }
    log('logins', {
      outcome: report.logins,
      imported: report.cookies.imported,
      sites: report.cookies.sites,
      skipped: report.cookies.skipped,
      failedBySite: report.cookies.failedBySite,
    });
  }

  if (choices.bookmarks) {
    const bookmarks = readChromeBookmarks(files.bookmarks);
    report.bookmarks = targets.addBookmarks(bookmarks);
    log('bookmarks', { imported: report.bookmarks });
  }

  if (choices.history && !fs.existsSync(files.history)) {
    report.history = 0;
    log('history', { imported: 0 });
  } else if (choices.history) {
    try {
      const history = withCopiedDatabase(
        files.history,
        (db) => readChromeHistory(db, options.nowMs),
        tempRoot
      );
      report.history = targets.addHistory(history);
    } catch (error) {
      if (!(error instanceof UnreadableDatabaseError)) throw error;
      report.history = -1;
    }
    log('history', { imported: report.history });
  }

  if (choices.settings) {
    report.settings = targets.applySettings(readChromePreferences(files.preferences));
    log('settings', { applied: report.settings });
  }

  return report;
}
