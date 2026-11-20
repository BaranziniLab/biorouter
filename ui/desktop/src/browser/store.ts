import fs from 'node:fs';
import path from 'node:path';

/**
 * The Biorouter Browser's own history and bookmarks (BB-22, phase 1).
 *
 * Two small JSON files in the app's user data folder, written with mode 0600. They exist for the
 * person: the address field's suggestions and the toolbar's Bookmarks menu read them. Nothing here
 * is sent to the daemon, a model, a log or the transcript ("Nothing about the person's own browsing
 * is written to the transcript").
 *
 * History keeps 90 days and at most 5,000 pages. A page whose query carries a one-time credential
 * (an OAuth `code`, an access token, a signature) is not recorded at all: it is never worth
 * suggesting again, and it should not sit on disk.
 */

export type HistoryEntry = { url: string; title: string; visits: number; lastVisit: number };
export type Bookmark = { url: string; title: string; addedAt: number };
export type Suggestion = { url: string; title: string; bookmarked: boolean };
/** A bookmark an import brought, under its folder ("Imported from Chrome (Work)", BB-22). */
export type ImportedBookmark = Bookmark & { folder: string[] };
/** One import's bookmarks and history, kept apart so removal takes exactly them (BB-25). */
export type ImportedBrowsingData = {
  label: string;
  bookmarks: ImportedBookmark[];
  history: HistoryEntry[];
};
export type ImportedBookmarkFolder = {
  importId: string;
  label: string;
  bookmarks: ImportedBookmark[];
};

export const HISTORY_RETENTION_MS = 90 * 24 * 60 * 60 * 1000;
export const MAX_HISTORY_ENTRIES = 5_000;
export const MAX_BOOKMARKS = 1_000;
/** Per import: Chrome profiles hold more than the person's own list here ever will. */
export const MAX_IMPORTED_BOOKMARKS = 5_000;
export const MAX_IMPORTED_HISTORY = 10_000;
const MAX_URL_CHARS = 2_048;
const MAX_TITLE_CHARS = 300;
const WRITE_DELAY_MS = 1_000;

const SENSITIVE_QUERY_KEYS = new Set([
  'code',
  'token',
  'access_token',
  'id_token',
  'refresh_token',
  'client_secret',
  'password',
  'passwd',
  'secret',
  'sig',
  'signature',
  'x-amz-signature',
  'x-amz-credential',
  'sessionid',
  'otp',
  'ticket',
]);

/** Whether a page belongs in history at all. */
export function isRecordableUrl(candidate: string): boolean {
  if (candidate.length > MAX_URL_CHARS) return false;
  try {
    const url = new URL(candidate);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return false;
    if (url.username || url.password) return false;
    for (const key of url.searchParams.keys()) {
      if (SENSITIVE_QUERY_KEYS.has(key.toLowerCase())) return false;
    }
    return true;
  } catch {
    return false;
  }
}

function cleanTitle(title: string, url: string): string {
  const trimmed = title.replace(/\s+/g, ' ').trim().slice(0, MAX_TITLE_CHARS);
  return trimmed || url;
}

function readJson<T>(file: string, fallback: T): T {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8')) as T;
  } catch {
    return fallback;
  }
}

export type BrowserStore = ReturnType<typeof createBrowserStore>;

export function createBrowserStore(
  dir: string,
  options: { persist?: boolean; now?: () => number } = {}
) {
  const persist = options.persist !== false;
  const now = options.now ?? (() => Date.now());
  const historyFile = path.join(dir, 'history.json');
  const bookmarksFile = path.join(dir, 'bookmarks.json');
  const importedFile = path.join(dir, 'imported.json');
  const history = new Map<string, HistoryEntry>();
  let bookmarks: Bookmark[] = [];
  const imported = new Map<string, ImportedBrowsingData>();
  let loaded = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let dirtyHistory = false;
  let dirtyBookmarks = false;
  let dirtyImported = false;

  const load = () => {
    if (loaded) return;
    loaded = true;
    if (!persist) return;
    const storedHistory = readJson<{ entries?: HistoryEntry[] }>(historyFile, {});
    for (const entry of storedHistory.entries ?? []) {
      if (
        entry &&
        typeof entry.url === 'string' &&
        typeof entry.title === 'string' &&
        typeof entry.lastVisit === 'number' &&
        isRecordableUrl(entry.url)
      ) {
        history.set(entry.url, {
          url: entry.url,
          title: cleanTitle(entry.title, entry.url),
          visits: Math.max(1, Math.floor(Number(entry.visits) || 1)),
          lastVisit: entry.lastVisit,
        });
      }
    }
    const storedBookmarks = readJson<{ bookmarks?: Bookmark[] }>(bookmarksFile, {});
    bookmarks = (storedBookmarks.bookmarks ?? [])
      .filter(
        (bookmark) =>
          bookmark && typeof bookmark.url === 'string' && typeof bookmark.title === 'string'
      )
      .slice(0, MAX_BOOKMARKS)
      .map((bookmark) => ({
        url: bookmark.url,
        title: cleanTitle(bookmark.title, bookmark.url),
        addedAt: Number(bookmark.addedAt) || 0,
      }));
    const storedImports = readJson<{ imports?: Record<string, ImportedBrowsingData> }>(
      importedFile,
      {}
    );
    for (const [importId, data] of Object.entries(storedImports.imports ?? {})) {
      if (!data || typeof data !== 'object') continue;
      imported.set(importId, {
        label: typeof data.label === 'string' ? data.label : 'Imported',
        bookmarks: cleanImportedBookmarks(data.bookmarks),
        history: cleanImportedHistory(data.history),
      });
    }
    prune();
  };

  const cleanImportedBookmarks = (items: unknown): ImportedBookmark[] =>
    (Array.isArray(items) ? items : [])
      .filter(
        (item): item is ImportedBookmark =>
          item &&
          typeof item.url === 'string' &&
          typeof item.title === 'string' &&
          isRecordableUrl(item.url)
      )
      .slice(0, MAX_IMPORTED_BOOKMARKS)
      .map((item) => ({
        url: item.url,
        title: cleanTitle(item.title, item.url),
        addedAt: Number(item.addedAt) || 0,
        folder: (Array.isArray(item.folder) ? item.folder : [])
          .filter((name): name is string => typeof name === 'string')
          .slice(0, 16)
          .map((name) => name.slice(0, MAX_TITLE_CHARS)),
      }));

  const cleanImportedHistory = (items: unknown): HistoryEntry[] => {
    const cutoff = now() - HISTORY_RETENTION_MS;
    const seen = new Set<string>();
    return (Array.isArray(items) ? items : [])
      .filter(
        (item): item is HistoryEntry =>
          item &&
          typeof item.url === 'string' &&
          typeof item.lastVisit === 'number' &&
          item.lastVisit >= cutoff &&
          isRecordableUrl(item.url)
      )
      .filter((item) => !seen.has(item.url) && Boolean(seen.add(item.url)))
      .slice(0, MAX_IMPORTED_HISTORY)
      .map((item) => ({
        url: item.url,
        title: cleanTitle(typeof item.title === 'string' ? item.title : '', item.url),
        visits: Math.max(1, Math.floor(Number(item.visits) || 1)),
        lastVisit: item.lastVisit,
      }));
  };

  const prune = () => {
    const cutoff = now() - HISTORY_RETENTION_MS;
    for (const [url, entry] of history) {
      if (entry.lastVisit < cutoff) history.delete(url);
    }
    if (history.size > MAX_HISTORY_ENTRIES) {
      const ordered = [...history.values()].sort((a, b) => a.lastVisit - b.lastVisit);
      for (const entry of ordered.slice(0, history.size - MAX_HISTORY_ENTRIES)) {
        history.delete(entry.url);
      }
    }
  };

  const writeFile = (file: string, value: unknown) => {
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
    const temporary = `${file}.${process.pid}.tmp`;
    fs.writeFileSync(temporary, JSON.stringify(value), { mode: 0o600 });
    fs.renameSync(temporary, file);
  };

  const flush = () => {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    if (!persist) {
      dirtyHistory = false;
      dirtyBookmarks = false;
      dirtyImported = false;
      return;
    }
    try {
      if (dirtyHistory) {
        dirtyHistory = false;
        writeFile(historyFile, { version: 1, entries: [...history.values()] });
      }
      if (dirtyBookmarks) {
        dirtyBookmarks = false;
        writeFile(bookmarksFile, { version: 1, bookmarks });
      }
      if (dirtyImported) {
        dirtyImported = false;
        writeFile(importedFile, { version: 1, imports: Object.fromEntries(imported) });
      }
    } catch {
      // A full disk or a read only profile loses history, not the page.
    }
  };

  const schedule = () => {
    if (timer) return;
    timer = setTimeout(flush, WRITE_DELAY_MS);
    timer.unref?.();
  };

  return {
    recordVisit(url: string, title: string) {
      load();
      if (!isRecordableUrl(url)) return;
      const existing = history.get(url);
      history.delete(url);
      history.set(url, {
        url,
        title: cleanTitle(title || existing?.title || '', url),
        visits: (existing?.visits ?? 0) + 1,
        lastVisit: now(),
      });
      prune();
      dirtyHistory = true;
      schedule();
    },

    updateTitle(url: string, title: string) {
      load();
      const existing = history.get(url);
      if (!existing || !title.trim()) return;
      existing.title = cleanTitle(title, url);
      dirtyHistory = true;
      schedule();
    },

    suggest(query: string, limit = 8): Suggestion[] {
      load();
      const needle = query.trim().toLowerCase();
      if (!needle) return [];
      const importedBookmarks = [...imported.values()].flatMap((data) => data.bookmarks);
      const allBookmarks = [...bookmarks, ...importedBookmarks];
      const marked = new Set(allBookmarks.map((bookmark) => bookmark.url));
      const matches = (url: string, title: string) =>
        url.toLowerCase().includes(needle) || title.toLowerCase().includes(needle);
      const seenBookmarks = new Set<string>();
      const fromBookmarks = allBookmarks
        .filter((bookmark) => matches(bookmark.url, bookmark.title))
        .filter(
          (bookmark) => !seenBookmarks.has(bookmark.url) && Boolean(seenBookmarks.add(bookmark.url))
        )
        .map((bookmark) => ({ url: bookmark.url, title: bookmark.title, bookmarked: true }));
      const reference = now();
      // The person's own history first, then imported pages it does not already hold.
      const visited = new Map(history);
      for (const data of imported.values()) {
        for (const entry of data.history) {
          if (!visited.has(entry.url)) visited.set(entry.url, entry);
        }
      }
      const fromHistory = [...visited.values()]
        .filter((entry) => !marked.has(entry.url) && matches(entry.url, entry.title))
        .sort((a, b) => {
          const score = (entry: HistoryEntry) =>
            entry.visits / (1 + (reference - entry.lastVisit) / (24 * 60 * 60 * 1000));
          return score(b) - score(a);
        })
        .map((entry) => ({ url: entry.url, title: entry.title, bookmarked: false }));
      return [...fromBookmarks, ...fromHistory].slice(0, Math.max(0, Math.min(limit, 20)));
    },

    bookmarks(): Bookmark[] {
      load();
      return bookmarks.map((bookmark) => ({ ...bookmark }));
    },

    isBookmarked(url: string): boolean {
      load();
      return bookmarks.some((bookmark) => bookmark.url === url);
    },

    addBookmark(url: string, title: string): boolean {
      load();
      if (!isRecordableUrl(url) || bookmarks.some((bookmark) => bookmark.url === url)) return false;
      if (bookmarks.length >= MAX_BOOKMARKS) return false;
      bookmarks = [...bookmarks, { url, title: cleanTitle(title, url), addedAt: now() }];
      dirtyBookmarks = true;
      schedule();
      return true;
    },

    removeBookmark(url: string): boolean {
      load();
      const next = bookmarks.filter((bookmark) => bookmark.url !== url);
      if (next.length === bookmarks.length) return false;
      bookmarks = next;
      dirtyBookmarks = true;
      schedule();
      return true;
    },

    clearHistory() {
      load();
      history.clear();
      dirtyHistory = true;
      for (const data of imported.values()) data.history = [];
      dirtyImported = true;
      flush();
    },

    /** Bookmarks an import brought, one folder per import, for the Bookmarks menu. */
    importedBookmarkFolders(): ImportedBookmarkFolder[] {
      load();
      return [...imported]
        .filter(([, data]) => data.bookmarks.length > 0)
        .map(([importId, data]) => ({
          importId,
          label: data.label,
          bookmarks: data.bookmarks.map((bookmark) => ({
            ...bookmark,
            folder: [...bookmark.folder],
          })),
        }));
    },

    /** One import's bookmarks and history, copied. */
    importedData(importId: string): { bookmarks: ImportedBookmark[]; history: HistoryEntry[] } {
      load();
      const data = imported.get(importId);
      return {
        bookmarks: (data?.bookmarks ?? []).map((bookmark) => ({
          ...bookmark,
          folder: [...bookmark.folder],
        })),
        history: (data?.history ?? []).map((entry) => ({ ...entry })),
      };
    },

    /** Keep one import's bookmarks under `label`; returns how many were kept. */
    importBookmarks(importId: string, label: string, items: ImportedBookmark[]): number {
      load();
      const existing = imported.get(importId) ?? { label, bookmarks: [], history: [] };
      existing.label = label;
      existing.bookmarks = cleanImportedBookmarks(items);
      imported.set(importId, existing);
      dirtyImported = true;
      flush();
      return existing.bookmarks.length;
    },

    /** Keep one import's history (the last 90 days); returns how many pages were kept. */
    importHistory(importId: string, label: string, items: HistoryEntry[]): number {
      load();
      const existing = imported.get(importId) ?? { label, bookmarks: [], history: [] };
      existing.label = label;
      existing.history = cleanImportedHistory(items);
      imported.set(importId, existing);
      dirtyImported = true;
      flush();
      return existing.history.length;
    },

    /** Remove everything one import brought (BB-25); the person's own data is untouched. */
    removeImport(importId: string): { bookmarks: number; history: number } {
      load();
      const data = imported.get(importId);
      if (!data) return { bookmarks: 0, history: 0 };
      imported.delete(importId);
      dirtyImported = true;
      flush();
      return { bookmarks: data.bookmarks.length, history: data.history.length };
    },

    flush,
  };
}
