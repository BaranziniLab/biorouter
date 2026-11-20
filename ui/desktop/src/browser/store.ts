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

export const HISTORY_RETENTION_MS = 90 * 24 * 60 * 60 * 1000;
export const MAX_HISTORY_ENTRIES = 5_000;
export const MAX_BOOKMARKS = 1_000;
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
  const history = new Map<string, HistoryEntry>();
  let bookmarks: Bookmark[] = [];
  let loaded = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let dirtyHistory = false;
  let dirtyBookmarks = false;

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
    prune();
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
      const marked = new Set(bookmarks.map((bookmark) => bookmark.url));
      const matches = (url: string, title: string) =>
        url.toLowerCase().includes(needle) || title.toLowerCase().includes(needle);
      const fromBookmarks = bookmarks
        .filter((bookmark) => matches(bookmark.url, bookmark.title))
        .map((bookmark) => ({ url: bookmark.url, title: bookmark.title, bookmarked: true }));
      const reference = now();
      const fromHistory = [...history.values()]
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
      flush();
    },

    flush,
  };
}
