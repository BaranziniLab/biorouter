import fs from 'node:fs';
import type { DatabaseSync } from 'node:sqlite';
import { tableColumns } from './sqliteCopy';
import { CHROME_EPOCH_OFFSET_SECONDS } from './cookies';

/**
 * Chrome's bookmarks, history and the few preferences the Biorouter Browser takes (BB-22).
 *
 * Bookmarks and Preferences are JSON files Chrome replaces atomically, so they are read in place.
 * History is SQLite and is read from a copy by the caller (`withCopiedDatabase`).
 */

const MAX_JSON_BYTES = 32 * 1024 * 1024;
export const MAX_IMPORTED_BOOKMARKS = 5_000;
export const MAX_IMPORTED_HISTORY = 10_000;
export const HISTORY_IMPORT_DAYS = 90;
const MAX_URL_CHARS = 2_048;
const MAX_TITLE_CHARS = 300;

export type ChromeBookmark = {
  url: string;
  title: string;
  /** Folder names from the root, for example `["Bookmarks bar", "Research"]`. */
  folder: string[];
  /** Milliseconds since 1970, 0 when Chrome recorded none. */
  addedAt: number;
};

export type ChromeHistoryEntry = {
  url: string;
  title: string;
  visits: number;
  typed: number;
  /** Milliseconds since 1970. */
  lastVisit: number;
};

export type SearchEngine = { name: string; template: string };

export type ChromePreferences = {
  searchEngine: SearchEngine | null;
  acceptLanguages: string | null;
  /** Chrome's default zoom as a factor (1 is 100%), or null when it is the default. */
  defaultZoomFactor: number | null;
};

function readJson(file: string): unknown {
  try {
    const stat = fs.statSync(file);
    if (!stat.isFile() || stat.size > MAX_JSON_BYTES) return null;
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch {
    return null;
  }
}

function webUrl(candidate: unknown): string | null {
  if (typeof candidate !== 'string' || candidate.length > MAX_URL_CHARS) return null;
  try {
    const url = new URL(candidate);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
    if (url.username || url.password) return null;
    return url.href;
  } catch {
    return null;
  }
}

function cleanTitle(value: unknown): string {
  return typeof value === 'string'
    ? value.replace(/\s+/g, ' ').trim().slice(0, MAX_TITLE_CHARS)
    : '';
}

/** Chrome's microseconds since 1601, as a decimal string or number, to milliseconds since 1970. */
export function chromeMicrosToUnixMs(value: unknown): number {
  let micros: bigint;
  try {
    if (typeof value === 'bigint') micros = value;
    else if (typeof value === 'number' && Number.isFinite(value))
      micros = BigInt(Math.trunc(value));
    else if (typeof value === 'string' && /^\d{1,20}$/.test(value)) micros = BigInt(value);
    else return 0;
  } catch {
    return 0;
  }
  if (micros <= 0n) return 0;
  const ms = Number(micros / 1000n) - CHROME_EPOCH_OFFSET_SECONDS * 1000;
  return ms > 0 ? ms : 0;
}

const ROOT_NAMES: Record<string, string> = {
  bookmark_bar: 'Bookmarks bar',
  other: 'Other bookmarks',
  synced: 'Mobile bookmarks',
};

/** Every `url` bookmark in Chrome's `Bookmarks` file, folders kept as a path. */
export function readChromeBookmarks(file: string): ChromeBookmark[] {
  const parsed = readJson(file) as { roots?: Record<string, unknown> } | null;
  const roots = parsed && typeof parsed.roots === 'object' && parsed.roots ? parsed.roots : {};
  const out: ChromeBookmark[] = [];
  const seen = new Set<string>();

  const walk = (node: unknown, folder: string[], depth: number) => {
    if (out.length >= MAX_IMPORTED_BOOKMARKS || depth > 32) return;
    if (!node || typeof node !== 'object') return;
    const item = node as Record<string, unknown>;
    if (item.type === 'url') {
      const url = webUrl(item.url);
      if (!url) return;
      const key = `${folder.join('\u0000')}\u0001${url}`;
      if (seen.has(key)) return;
      seen.add(key);
      out.push({
        url,
        title: cleanTitle(item.name) || url,
        folder,
        addedAt: chromeMicrosToUnixMs(item.date_added),
      });
      return;
    }
    if (item.type === 'folder' && Array.isArray(item.children)) {
      const name = cleanTitle(item.name);
      const next = depth === 0 ? folder : [...folder, name || 'Folder'];
      for (const child of item.children) walk(child, next, depth + 1);
    }
  };

  for (const [rootKey, root] of Object.entries(roots)) {
    const label = ROOT_NAMES[rootKey];
    if (!label) continue;
    walk(root, [label], 0);
  }
  return out;
}

/**
 * Chrome's history, newest first: the `urls` table's rows visited in the last `days`, without
 * hidden rows and without anything but `http` and `https`. Never the `visits` table's detail.
 */
export function readChromeHistory(
  db: DatabaseSync,
  nowMs: number,
  days = HISTORY_IMPORT_DAYS
): ChromeHistoryEntry[] {
  const columns = tableColumns(db, 'urls');
  if (!columns.has('url') || !columns.has('last_visit_time')) return [];
  const cutoffMicros =
    (BigInt(Math.trunc(nowMs - days * 24 * 60 * 60 * 1000)) +
      BigInt(CHROME_EPOCH_OFFSET_SECONDS) * 1000n) *
    1000n;
  const hidden = columns.has('hidden') ? 'AND hidden = 0' : '';
  const statement = db.prepare(
    `SELECT url, ${columns.has('title') ? 'title' : "''"} AS title,
       ${columns.has('visit_count') ? 'visit_count' : '1'} AS visits,
       ${columns.has('typed_count') ? 'typed_count' : '0'} AS typed,
       last_visit_time / 1000 AS last_visit_ms_1601
     FROM urls
     WHERE last_visit_time >= ? ${hidden}
     ORDER BY last_visit_time DESC
     LIMIT ${MAX_IMPORTED_HISTORY * 2}`
  );
  const rows = statement.all(cutoffMicros) as Array<Record<string, unknown>>;
  const out: ChromeHistoryEntry[] = [];
  const seen = new Set<string>();
  for (const row of rows) {
    const url = webUrl(row.url);
    if (!url || seen.has(url)) continue;
    seen.add(url);
    const lastVisit = Number(row.last_visit_ms_1601) - CHROME_EPOCH_OFFSET_SECONDS * 1000;
    if (!(lastVisit > 0)) continue;
    out.push({
      url,
      title: cleanTitle(row.title) || url,
      visits: Math.max(1, Math.floor(Number(row.visits) || 1)),
      typed: Math.max(0, Math.floor(Number(row.typed) || 0)),
      lastVisit,
    });
    if (out.length >= MAX_IMPORTED_HISTORY) break;
  }
  return out;
}

const LANGUAGE = /^[a-z]{2,3}(?:-[a-z0-9]{2,8})*$/i;

function zoomFactorFromLevel(level: number): number | null {
  if (!Number.isFinite(level) || Math.abs(level) < 1e-6) return null;
  // Chrome's zoom level is log base 1.2 of the factor.
  const factor = Math.pow(1.2, level);
  if (factor < 0.25 || factor > 5) return null;
  return Math.round(factor * 100) / 100;
}

/** The search engine, languages and default zoom from Chrome's `Preferences` file. */
export function readChromePreferences(file: string): ChromePreferences {
  const parsed = readJson(file) as Record<string, unknown> | null;
  const prefs = parsed && typeof parsed === 'object' ? parsed : {};
  const record = (value: unknown) =>
    value && typeof value === 'object' ? (value as Record<string, unknown>) : {};

  let searchEngine: SearchEngine | null = null;
  const template = record(record(prefs.default_search_provider_data).template_url_data);
  const templateUrl = typeof template.url === 'string' ? template.url : '';
  if (templateUrl.includes('{searchTerms}') && templateUrl.length <= MAX_URL_CHARS) {
    const probe = templateUrl.replace('{searchTerms}', 'probe');
    try {
      const url = new URL(probe);
      if (url.protocol === 'https:' && !url.username && !url.password && !/\{[^}]*\}/.test(probe)) {
        searchEngine = {
          name: cleanTitle(template.short_name).slice(0, 60) || url.hostname,
          template: templateUrl,
        };
      }
    } catch {
      searchEngine = null;
    }
  }

  let acceptLanguages: string | null = null;
  const languages = record(prefs.intl).accept_languages;
  if (typeof languages === 'string') {
    const cleaned = [
      ...new Set(
        languages
          .split(',')
          .map((language) => language.trim())
          .filter((language) => LANGUAGE.test(language))
      ),
    ].slice(0, 8);
    if (cleaned.length > 0) acceptLanguages = cleaned.join(',');
  }

  let defaultZoomFactor: number | null = null;
  const partitionZoom = record(prefs.partition).default_zoom_level;
  if (typeof partitionZoom === 'number') {
    defaultZoomFactor = zoomFactorFromLevel(partitionZoom);
  } else if (partitionZoom && typeof partitionZoom === 'object') {
    // Keyed by storage partition; the default partition's key is `x`.
    const levels = partitionZoom as Record<string, unknown>;
    const level =
      typeof levels.x === 'number' ? levels.x : Object.values(levels).find(Number.isFinite);
    if (typeof level === 'number') defaultZoomFactor = zoomFactorFromLevel(level);
  } else if (typeof record(prefs.profile).default_zoom_level === 'number') {
    defaultZoomFactor = zoomFactorFromLevel(record(prefs.profile).default_zoom_level as number);
  }

  return { searchEngine, acceptLanguages, defaultZoomFactor };
}
