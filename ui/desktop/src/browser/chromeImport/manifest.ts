import fs from 'node:fs';
import path from 'node:path';
import type { BrowserPreferences } from '../preferences';
import type { ChromeImportSummary } from '../types';
import type { ImportedCookieKey } from './cookies';
import type { AppliedSettings, LoginsOutcome } from './importer';
import type { KeyFailure } from './keyProvider';

/**
 * What each import brought, so **Settings > App > Browser** can say it and **Remove imported data**
 * can take it back (BB-25). `import-manifest.json` beside the browser's history, mode 0600.
 *
 * ⚠ It holds the KEYS of the cookies an import set (url, name, domain, path) and never a value.
 * A test reads the file after a fixture import and fails if any fixture value appears in it.
 */

export type ImportRecord = {
  id: string;
  source: 'chrome';
  profileId: string;
  profileName: string;
  importedAt: number;
  logins: LoginsOutcome;
  /** Absent in imports recorded before it existed. */
  loginsMissingKey?: KeyFailure | null;
  cookies: number;
  cookieSites: number;
  skipped: number;
  failed: number;
  bookmarks: number | null;
  history: number | null;
  settings: AppliedSettings | null;
  cookieKeys: ImportedCookieKey[];
  /** The settings as they were before this profile's first import, restored on removal. */
  previousSettings: BrowserPreferences;
};

export type ImportManifest = {
  version: 1;
  /** **Not now** on the first use card: never offer it again. */
  offerDismissed: boolean;
  imports: ImportRecord[];
};

export const MANIFEST_FILE = 'import-manifest.json';

const EMPTY: ImportManifest = { version: 1, offerDismissed: false, imports: [] };

function validKey(value: unknown): value is ImportedCookieKey {
  if (!value || typeof value !== 'object') return false;
  const key = value as Record<string, unknown>;
  return (
    typeof key.url === 'string' &&
    /^https?:\/\//.test(key.url) &&
    typeof key.name === 'string' &&
    typeof key.path === 'string' &&
    (key.domain === undefined || typeof key.domain === 'string')
  );
}

function validRecord(value: unknown): value is ImportRecord {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.id === 'string' &&
    record.source === 'chrome' &&
    typeof record.profileId === 'string' &&
    typeof record.profileName === 'string' &&
    typeof record.importedAt === 'number' &&
    Array.isArray(record.cookieKeys)
  );
}

export function readManifest(dir: string | null): ImportManifest {
  if (!dir) return { ...EMPTY, imports: [] };
  try {
    const parsed = JSON.parse(fs.readFileSync(path.join(dir, MANIFEST_FILE), 'utf8')) as Record<
      string,
      unknown
    >;
    const imports = (Array.isArray(parsed.imports) ? parsed.imports : [])
      .filter(validRecord)
      .map((record) => ({
        ...record,
        cookieKeys: record.cookieKeys.filter(validKey),
        previousSettings:
          record.previousSettings && typeof record.previousSettings === 'object'
            ? record.previousSettings
            : {},
      }));
    return { version: 1, offerDismissed: parsed.offerDismissed === true, imports };
  } catch {
    return { ...EMPTY, imports: [] };
  }
}

export function writeManifest(dir: string | null, manifest: ImportManifest): void {
  if (!dir) return;
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  const file = path.join(dir, MANIFEST_FILE);
  const temporary = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(temporary, JSON.stringify(manifest), { mode: 0o600 });
  fs.renameSync(temporary, file);
}

/** What the renderer is told about an import: counts and names, no cookie keys. */
export function summarize(record: ImportRecord): ChromeImportSummary {
  return {
    id: record.id,
    profileName: record.profileName,
    importedAt: record.importedAt,
    logins: record.logins,
    loginsMissingKey: record.loginsMissingKey ?? null,
    cookies: record.cookies,
    cookieSites: record.cookieSites,
    skipped: record.skipped,
    failed: record.failed,
    bookmarks: record.bookmarks,
    history: record.history,
    settings: record.settings,
  };
}
