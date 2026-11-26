/**
 * The **History** tab's data (DD-96; "Data and live updates" in `history-design.md` 10.5 and
 * "Dictation history" in `docs/dictation/surfaces.md`): the dictations this caller may list,
 * newest first, the history settings with their `authority`, and every write, over the
 * dictation history routes (`GET /dictation/history` and its siblings) through the generated
 * client.
 *
 * - **Every call sends `userActionHeaders()`**: the proof of a person on the desktop, the
 *   host's provider on `serve`. Without it the daemon lists only public dictations, so private
 *   ones would silently vanish from the tab.
 * - **Pages of 100, newest first.** The next page is read when the list nears its end
 *   ({@link loadMoreHistory}). A refresh reads again as many entries as the tab already holds,
 *   so a person who scrolled down keeps their place.
 * - **The search is the daemon's** (`q`, a case folded substring of the text), sent 200 ms
 *   after the last keystroke ({@link HISTORY_SEARCH_DEBOUNCE_MS}). The entries always answer
 *   {@link HistorySnapshot.query}, the search the daemon was asked, never the one being typed.
 * - **Stale answers are dropped.** Each read takes a generation number; a read that lands after
 *   a newer one started (a keystroke, a focus, a write) is thrown away.
 * - **`If-None-Match`.** Each page's tag is sent back on the next read and a `304` keeps the page
 *   held here. The daemon's CORS layer exposes no `ETag`, so the tag is derived from the body
 *   the way the daemon derives it (`etag_of` in `routes/dictation_history.rs`, the dictionary's
 *   formula, {@link derivedEtag}).
 * - **Live.** While the tab is open, the listing is read again on window focus, when the window
 *   becomes visible, every 60 s while it is visible, and after `dictationSession` says a
 *   transcript was published (`notifyDictationTranscribed`, about 500 ms after the text is out,
 *   because the daemon appends after it answers).
 * - **Confirmed state, never intent.** A write whose answer carries the entry (a tier change)
 *   or names it gone (a delete) puts that in place at once, then reads the listing again behind
 *   it; a clear and a settings write answer after the listing was read.
 */
import { useEffect, useRef, useSyncExternalStore } from 'react';
import {
  changeDictationHistorySettings,
  changeDictationHistoryTier,
  clearDictationHistory,
  deleteDictationHistoryEntry,
  getDictationHistorySettings,
  listDictationHistory,
} from '../../api';
import type {
  DictationHistoryDeleted,
  DictationHistoryEntryAnswer,
  DictationHistoryEntryView,
  DictationHistoryList,
  DictationHistorySettings,
  DictationHistorySettingsRequest,
  DictionaryTier,
} from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { subscribeDictationTranscribed } from '../dictation/dictationSession';
import {
  derivedEtag,
  failureOf,
  installLiveRefresh,
  type DictionaryFailure,
  type Outcome,
} from './useDictionary';

/** Entries one listing page asks for (the daemon's default, `DEFAULT_PAGE`). */
export const HISTORY_PAGE_SIZE = 100;
/** The daemon's longest search, in characters (`MAX_QUERY_CHARS`); a longer one is cut. */
export const HISTORY_MAX_QUERY_CHARS = 200;
/** How long after the last keystroke the search is sent. */
export const HISTORY_SEARCH_DEBOUNCE_MS = 200;
/** The history's own cap is 5,000 entries: fifty pages of 100. */
const MAX_PAGES = 50;

export type HistoryFailure = DictionaryFailure;
export type HistoryLoad = 'unloaded' | 'loading' | 'loaded' | 'failed';

export interface HistorySnapshot {
  load: HistoryLoad;
  /** The search the entries answer: what the daemon was asked, trimmed. */
  query: string;
  /** Every page read so far, newest first. */
  entries: DictationHistoryEntryView[];
  /** How many entries this caller may list match `query`, over every page. */
  total: number;
  /** The next page's cursor; null when every match is held. */
  nextCursor: string | null;
  settings: DictationHistorySettings | null;
  /** The last read's refusal: MSG-HISTORY-FILE (`unreadable`), MSG-REMOTE-OLD, or why it failed. */
  error: HistoryFailure | null;
  /** The daemon answered 404 or 405: a backend from before the history. */
  absent: boolean;
  /**
   * What the last read without a search said of the whole history: how many entries this caller
   * may list, and the newest one's time. **Clear history…** deletes through that time, so a
   * dictation recorded while its confirmation is open survives.
   */
  everything: { total: number; newestAt: string | null };
}

const INITIAL: HistorySnapshot = {
  load: 'unloaded',
  query: '',
  entries: [],
  total: 0,
  nextCursor: null,
  settings: null,
  error: null,
  absent: false,
  everything: { total: 0, newestAt: null },
};

let snapshot: HistorySnapshot = INITIAL;
const listeners = new Set<() => void>();

function publish(next: Partial<HistorySnapshot>): void {
  snapshot = { ...snapshot, ...next };
  for (const listener of [...listeners]) listener();
}

export function getHistorySnapshot(): HistorySnapshot {
  return snapshot;
}

export function subscribeHistory(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

// ── results ──────────────────────────────────────────────────────────────────────────

interface RawResult {
  data?: unknown;
  error?: unknown;
  response?: Response;
}

async function settle<T>(call: () => Promise<unknown>): Promise<Outcome<T>> {
  let result: RawResult;
  try {
    result = (await call()) as RawResult;
  } catch {
    result = {};
  }
  if (result.response && result.response.ok) {
    return { ok: true, status: result.response.status, data: result.data as T };
  }
  return { ok: false, error: failureOf(result) };
}

/** A search as the daemon takes it: trimmed, at most 200 characters. */
export function historyQueryOf(text: string): string {
  return Array.from(text.trim()).slice(0, HISTORY_MAX_QUERY_CHARS).join('').trim();
}

// ── the listing ──────────────────────────────────────────────────────────────────────

interface CachedPage {
  etag: string | null;
  text: string;
  page: DictationHistoryList;
}

let pageCache = new Map<string, CachedPage>();
let generation = 0;
/** The search the newest read asked for, which a refresh repeats while that read is out. */
let requested = '';

type PageRead =
  | { ok: true; page: DictationHistoryList; changed: boolean }
  | { ok: false; error: HistoryFailure };

/**
 * One page of `GET /dictation/history`, sent with the page's last tag. A `304` answers the page
 * held in `cache` (unchanged); a `200` replaces it.
 */
async function readPage(
  cache: Map<string, CachedPage>,
  query: string,
  cursor: string | null,
  headers: Record<string, string>
): Promise<PageRead> {
  const key = `${query}\u0000${cursor ?? ''}`;
  const held = cache.get(key);
  let result: RawResult;
  try {
    result = (await listDictationHistory({
      query: {
        ...(query ? { q: query } : {}),
        ...(cursor ? { cursor } : {}),
        limit: HISTORY_PAGE_SIZE,
      },
      headers: { ...headers, ...(held?.etag ? { 'If-None-Match': held.etag } : {}) },
      parseAs: 'text',
      // A conditional request is answered by the daemon, not by the HTTP cache.
      cache: 'no-store',
    })) as RawResult;
  } catch {
    result = {};
  }
  const response = result.response;
  if (response?.status === 304 && held) return { ok: true, page: held.page, changed: false };
  if (!response?.ok) return { ok: false, error: failureOf(result) };
  const text = typeof result.data === 'string' ? result.data : '';
  if (held && held.text === text) return { ok: true, page: held.page, changed: false };
  let page: DictationHistoryList;
  try {
    page = JSON.parse(text) as DictationHistoryList;
  } catch {
    return { ok: false, error: failureOf({ response, error: {} }) };
  }
  if (!Array.isArray(page?.entries))
    return { ok: false, error: failureOf({ response, error: {} }) };
  const etag = response.headers.get('ETag') ?? (await derivedEtag(text));
  cache.set(key, { etag, text, page });
  return { ok: true, page, changed: true };
}

function sameSettings(
  a: DictationHistorySettings | null,
  b: DictationHistorySettings | null
): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * Read `query` from its first page, as many entries as `want` (at least one page), and the
 * settings. Resolves when the read has been published or dropped.
 */
async function readListing(query: string, want: number): Promise<void> {
  const issued = ++generation;
  requested = query;
  const cache = pageCache;
  if (snapshot.load === 'unloaded' || snapshot.load === 'failed') publish({ load: 'loading' });
  const headers = await userActionHeaders();
  const settingsRead = settle<DictationHistorySettings>(() =>
    getDictationHistorySettings({ headers })
  );
  const entries: DictationHistoryEntryView[] = [];
  let failure: HistoryFailure | null = null;
  let changed = false;
  let total = 0;
  let cursor: string | null = null;
  let pages = 0;
  for (; pages < MAX_PAGES; pages++) {
    const read = await readPage(cache, query, cursor, headers);
    // A newer read has started: this one would be dropped, so stop asking.
    if (issued !== generation) return;
    if (!read.ok) {
      failure = read.error;
      break;
    }
    changed ||= read.changed;
    entries.push(...read.page.entries);
    total = read.page.total;
    cursor = read.page.next_cursor ?? null;
    if (!cursor || entries.length >= want) break;
  }
  const settings = await settingsRead;
  if (issued !== generation) return;
  const nextSettings = settings.ok ? settings.data : snapshot.settings;
  const settingsSame = sameSettings(nextSettings, snapshot.settings);
  if (failure) {
    publish({
      load: snapshot.load === 'loaded' && snapshot.query === query ? 'loaded' : 'failed',
      query,
      ...(snapshot.query === query ? {} : { entries: [], total: 0, nextCursor: null }),
      error: failure,
      absent: failure.status === 404 || failure.status === 405,
      ...(settings.ok && !settingsSame ? { settings: settings.data } : {}),
    });
    return;
  }
  const everything =
    query === '' ? { total, newestAt: entries[0]?.created_at ?? null } : snapshot.everything;
  // Every page answered unchanged, with as many entries as before: keep the very same rows, so
  // a quiet minute renders nothing.
  const unchanged =
    !changed &&
    snapshot.load === 'loaded' &&
    snapshot.query === query &&
    entries.length === snapshot.entries.length;
  if (
    unchanged &&
    settingsSame &&
    snapshot.error === null &&
    !snapshot.absent &&
    snapshot.total === total &&
    snapshot.nextCursor === cursor
  ) {
    return;
  }
  publish({
    load: 'loaded',
    query,
    entries: unchanged ? snapshot.entries : entries,
    total,
    nextCursor: cursor,
    error: null,
    absent: false,
    settings: settingsSame ? snapshot.settings : nextSettings,
    everything:
      everything.total === snapshot.everything.total &&
      everything.newestAt === snapshot.everything.newestAt
        ? snapshot.everything
        : everything,
  });
}

/**
 * Read the current search again (the newest one asked for): as many entries as the tab holds for
 * it, and the settings.
 */
export function refreshHistory(): Promise<void> {
  const held = requested === snapshot.query ? snapshot.entries.length : 0;
  return readListing(requested, Math.max(HISTORY_PAGE_SIZE, held));
}

/** Ask the daemon for `text` (trimmed, at most 200 characters), from its first page. */
export function searchHistory(text: string): Promise<void> {
  return readListing(historyQueryOf(text), HISTORY_PAGE_SIZE);
}

let loadingMore = false;

/**
 * Read the page after the last one held and append it. Does nothing while every match is held,
 * while another page is being read, or before the first read; dropped when a newer read started.
 */
export async function loadMoreHistory(): Promise<void> {
  const cursor = snapshot.nextCursor;
  if (!cursor || loadingMore || snapshot.load !== 'loaded') return;
  loadingMore = true;
  const issued = generation;
  const query = snapshot.query;
  try {
    const read = await readPage(pageCache, query, cursor, await userActionHeaders());
    if (issued !== generation || !read.ok) return;
    // An entry the person deleted meanwhile, or one already held, is not added twice.
    const held = new Set(snapshot.entries.map((entry) => entry.id));
    publish({
      entries: [...snapshot.entries, ...read.page.entries.filter((entry) => !held.has(entry.id))],
      total: read.page.total,
      nextCursor: read.page.next_cursor ?? null,
    });
  } finally {
    loadingMore = false;
  }
}

// ── writes ───────────────────────────────────────────────────────────────────────────

/** The listing read again after the last write that answered its entry; tests wait on it. */
let rereading: Promise<void> = Promise.resolve();

/** Put what a write answered into the listing at once: the entry as the daemon now holds it, or its id gone. */
function applyAnswered(answered: { entry: DictationHistoryEntryView } | { removed: string }): void {
  if (snapshot.load !== 'loaded') return;
  if ('removed' in answered) {
    const entries = snapshot.entries.filter((entry) => entry.id !== answered.removed);
    if (entries.length === snapshot.entries.length) return;
    publish({ entries, total: Math.max(0, snapshot.total - 1) });
    return;
  }
  const changed = answered.entry;
  publish({
    entries: snapshot.entries.map((entry) => (entry.id === changed.id ? changed : entry)),
  });
}

/** Delete one dictation (`DELETE /dictation/history/{id}`). */
export async function deleteHistoryEntry(id: string): Promise<Outcome<unknown>> {
  const headers = await userActionHeaders();
  const outcome = await settle<unknown>(() =>
    deleteDictationHistoryEntry({ path: { id }, headers })
  );
  if (outcome.ok) {
    applyAnswered({ removed: id });
    rereading = refreshHistory().catch(() => undefined);
    return outcome;
  }
  await refreshHistory();
  return outcome;
}

/** Make one dictation public or private (`POST /dictation/history/{id}/tier`). */
export async function setHistoryTier(
  id: string,
  tier: DictionaryTier
): Promise<Outcome<DictationHistoryEntryAnswer>> {
  const headers = await userActionHeaders();
  const outcome = await settle<DictationHistoryEntryAnswer>(() =>
    changeDictationHistoryTier({ path: { id }, body: { tier }, headers })
  );
  if (outcome.ok && outcome.data?.entry) {
    applyAnswered({ entry: outcome.data.entry });
    rereading = refreshHistory().catch(() => undefined);
    return outcome;
  }
  await refreshHistory();
  return outcome;
}

/**
 * **Clear history…**: every dictation this caller may list, created at or before the newest one
 * the last read without a search showed (`POST /dictation/history/clear`). Answers how many went.
 */
export async function clearHistory(): Promise<Outcome<DictationHistoryDeleted>> {
  const through = snapshot.everything.newestAt ?? snapshot.entries[0]?.created_at ?? null;
  if (!through) return { ok: true, status: 200, data: { deleted: 0 } };
  const headers = await userActionHeaders();
  const outcome = await settle<DictationHistoryDeleted>(() =>
    clearDictationHistory({ body: { through }, headers })
  );
  await refreshHistory();
  return outcome;
}

/**
 * Change **Dictation history** or **Save new dictations as public**
 * (`POST /dictation/history/settings`), then read the listing again.
 */
export async function setHistorySettings(
  change: DictationHistorySettingsRequest
): Promise<Outcome<DictationHistorySettings>> {
  const headers = await userActionHeaders();
  const outcome = await settle<DictationHistorySettings>(() =>
    changeDictationHistorySettings({ body: change, headers })
  );
  if (outcome.ok && outcome.data && !sameSettings(outcome.data, snapshot.settings)) {
    publish({ settings: outcome.data });
  }
  await refreshHistory();
  return outcome;
}

// ── the hook ─────────────────────────────────────────────────────────────────────────

export interface HistoryHook extends HistorySnapshot {
  refresh: typeof refreshHistory;
  loadMore: typeof loadMoreHistory;
  remove: typeof deleteHistoryEntry;
  setTier: typeof setHistoryTier;
  clear: typeof clearHistory;
  setSettings: typeof setHistorySettings;
}

/**
 * The History tab's data, read while `enabled` (the tab is open): on mount, then live (window
 * focus, visibility, every 60 s while visible, and after each published transcript). `query` is
 * the search field's text, sent {@link HISTORY_SEARCH_DEBOUNCE_MS} after it last changed.
 */
export function useDictationHistory({
  enabled,
  query,
}: {
  enabled: boolean;
  query: string;
}): HistoryHook {
  const current = useSyncExternalStore(subscribeHistory, getHistorySnapshot);
  const queryRef = useRef(query);
  queryRef.current = query;

  useEffect(() => {
    if (!enabled) return;
    // The tab opens on the search the field holds now; a search left over from before is
    // replaced at once rather than after the debounce.
    const typed = historyQueryOf(queryRef.current);
    if (typed !== requested) void searchHistory(typed);
    else void refreshHistory();
    const stopLive = installLiveRefresh(() => void refreshHistory());
    const stopTranscribed = subscribeDictationTranscribed(() => void refreshHistory());
    return () => {
      stopLive();
      stopTranscribed();
    };
  }, [enabled]);

  useEffect(() => {
    if (!enabled) return;
    const typed = historyQueryOf(query);
    if (typed === requested) return;
    const timer = setTimeout(() => void searchHistory(typed), HISTORY_SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [enabled, query]);

  return {
    ...current,
    refresh: refreshHistory,
    loadMore: loadMoreHistory,
    remove: deleteHistoryEntry,
    setTier: setHistoryTier,
    clear: clearHistory,
    setSettings: setHistorySettings,
  };
}

// ── tests ────────────────────────────────────────────────────────────────────────────

/** For tests: start over. A read still in flight from before is dropped when it lands. */
export function resetHistoryStoreForTests(): void {
  snapshot = INITIAL;
  listeners.clear();
  pageCache = new Map();
  generation += 1;
  requested = '';
  loadingMore = false;
  rereading = Promise.resolve();
}

/** For tests: the listing read again after the last write that answered its entry. */
export function historyRereadForTests(): Promise<void> {
  return rereading;
}
