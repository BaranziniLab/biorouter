/**
 * The Dictionary page's data ("Data and live updates" in
 * `docs/dictation/dictionary.md`): the rows this caller may list, the settings
 * answer with its `authority`, and every write, over PR-16's `/dictionary/*`
 * routes through the generated client.
 *
 * - **Every call sends `userActionHeaders()`**: the proof of a person on the
 *   desktop, the host's provider on `serve`. Without it the daemon lists only
 *   public rows, so private rows would silently vanish from the page.
 * - **Confirmed state, never intent.** Every write reads the listing again, so
 *   the page shows what the daemon holds. A write whose answer carries the row
 *   it changed (add, edit, tier change, delete) puts that row in place and
 *   resolves at once, while the listing is read again behind it; every other
 *   write, and every refusal, resolves after the listing has been read.
 * - **Stale responses are dropped.** Each read takes a generation number; a read
 *   that lands after a newer one started is thrown away.
 * - **`If-None-Match`.** Each page's ETag is sent back on the next read and a
 *   `304` keeps the page held here. The daemon's CORS layer does not expose
 *   `ETag` to the desktop renderer, whose origin differs from the daemon's, so
 *   when the header cannot be read the tag is derived from the body the way the
 *   daemon derives it (`etag_of` in `routes/dictionary.rs`: the quoted first 32
 *   hexadecimal digits of the SHA-256 of the page's bytes). A tag that no longer
 *   matches the daemon's formula only costs a `200`.
 * - **Live.** While a page mounts the hook, the listing is read again on window
 *   focus, when the window becomes visible, and every 60 s while it is visible.
 *
 * The sidebar's count is the same module's ({@link useDictionaryWaitingCount}),
 * so an accept or dismiss on the page moves the count at once.
 */
import { useEffect, useSyncExternalStore } from 'react';
import {
  acceptDictionarySuggestions,
  addDictionaryEntry,
  activateDictionary,
  changeDictionarySettings,
  changeDictionaryEntryTier,
  deleteDictionaryEntries,
  deleteDictionaryEntry,
  dismissDictionarySuggestions,
  editDictionaryEntry,
  exportDictionary,
  getDictionarySettings,
  importDictionary,
  keepDictionaryEntries,
  listDictionaryEntries,
  undoAgentDictionaryAdd,
} from '../../api';
import type {
  DictionaryAddRequest,
  DictionaryAgentWrites,
  DictionaryEditRequest,
  DictionaryEntryAnswer,
  DictionaryEntryList,
  DictionaryEntryView,
  DictionaryImportAnswer,
  DictionaryImportRequest,
  DictionarySettings,
  DictionaryStatusFilter,
  DictionaryTier,
} from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { dictationMessage, fillMessage } from '../dictation/copy';
import { remoteOldValues } from '../dictation/dictationStore';
import { dictionaryMessage } from './copy';

/** How often a visible page, and the sidebar's count, read the listing again. */
export const DICTIONARY_REFRESH_MS = 60_000;
/** `POST /dictionary/entries/delete` takes at most this many ids (`MAX_IDS`). */
export const MAX_IDS_PER_REQUEST = 5_000;

// ── results ──────────────────────────────────────────────────────────────────

/** A refusal or a failure, with the sentence to show. */
export interface DictionaryFailure {
  /** The HTTP status; 0 when the daemon did not answer. */
  status: number;
  code: string;
  message: string;
  /** `stale` only: the row's revision now. */
  rev?: number | null;
}

export type Outcome<T> =
  | { ok: true; status: number; data: T }
  | { ok: false; error: DictionaryFailure };

interface RawResult {
  data?: unknown;
  error?: unknown;
  response?: Response;
}

/** The sentence a refusal shows: the daemon's own, or one of the page's for an empty one. */
export function failureOf(result: RawResult): DictionaryFailure {
  const status = result.response?.status ?? 0;
  if (!result.response) {
    return {
      status: 0,
      code: 'unreachable',
      message: dictationMessage('MSG-SETTINGS-UNREACHABLE'),
    };
  }
  const body =
    result.error && typeof result.error === 'object'
      ? (result.error as { code?: unknown; message?: unknown; rev?: unknown })
      : {};
  const code = typeof body.code === 'string' && body.code ? body.code : `http_${status}`;
  const given = typeof body.message === 'string' ? body.message.trim() : '';
  const rev = typeof body.rev === 'number' ? body.rev : null;
  let message = given;
  if (!message) {
    if (code === 'too_large' || status === 413)
      message = dictionaryMessage('MSG-DICTIONARY-TOO-LARGE');
    else if (code === 'stale') message = dictionaryMessage('MSG-DICTIONARY-STALE');
    else if (status === 404 || status === 405)
      message = dictationMessage('MSG-REMOTE-OLD', remoteOldValues());
    else message = dictationMessage('MSG-SETTINGS-REFUSED', { code, status: String(status) });
  }
  return { status, code, message, ...(rev !== null ? { rev } : {}) };
}

async function settle<T>(call: Promise<RawResult>): Promise<Outcome<T>> {
  let result: RawResult;
  try {
    result = await call;
  } catch {
    result = {};
  }
  if (result.response && result.response.ok) {
    return { ok: true, status: result.response.status, data: result.data as T };
  }
  return { ok: false, error: failureOf(result) };
}

// ── the listing ──────────────────────────────────────────────────────────────

/** One page as it was last read, with the tag to send back. */
interface CachedPage {
  etag: string | null;
  text: string;
  page: DictionaryEntryList;
}

/** `"` + the first 32 hexadecimal digits of the SHA-256 of `text` + `"`, as the daemon tags a page. */
export async function derivedEtag(text: string): Promise<string | null> {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) return null;
  try {
    const digest = new Uint8Array(await subtle.digest('SHA-256', new TextEncoder().encode(text)));
    const hex = Array.from(digest.slice(0, 16), (byte) => byte.toString(16).padStart(2, '0')).join(
      ''
    );
    return `"${hex}"`;
  } catch {
    return null;
  }
}

export type PageRead =
  | { ok: true; page: DictionaryEntryList; changed: boolean }
  | { ok: false; error: DictionaryFailure };

/**
 * One page of `GET /dictionary/entries`, sent with the page's last tag. A `304`
 * answers the page held in `cache` (unchanged); a `200` replaces it.
 */
export async function readPage(
  cache: Map<string, CachedPage>,
  status: DictionaryStatusFilter,
  cursor: string | null,
  headers: Record<string, string>
): Promise<PageRead> {
  const key = `${status}:${cursor ?? ''}`;
  const held = cache.get(key);
  let result: RawResult;
  try {
    result = (await listDictionaryEntries({
      query: { status, ...(cursor ? { cursor } : {}) },
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
  let page: DictionaryEntryList;
  try {
    page = JSON.parse(text) as DictionaryEntryList;
  } catch {
    return { ok: false, error: failureOf({ response, error: {} }) };
  }
  const etag = response.headers.get('ETag') ?? (await derivedEtag(text));
  cache.set(key, { etag, text, page });
  return { ok: true, page, changed: true };
}

// ── the store ────────────────────────────────────────────────────────────────

export type DictionaryLoad = 'unloaded' | 'loading' | 'loaded' | 'failed';

export interface DictionarySnapshot {
  load: DictionaryLoad;
  /** Every page, merged, newest change first. */
  entries: DictionaryEntryView[];
  settings: DictionarySettings | null;
  /** MSG-DICTIONARY-FILE, MSG-REMOTE-OLD or the reason the last read failed. */
  error: string | null;
  /** The daemon answered 404 or 405: a backend from before the dictionary (S17). */
  absent: boolean;
}

const INITIAL: DictionarySnapshot = {
  load: 'unloaded',
  entries: [],
  settings: null,
  error: null,
  absent: false,
};

let snapshot: DictionarySnapshot = INITIAL;
const listeners = new Set<() => void>();
let pageCache = new Map<string, CachedPage>();
let generation = 0;

function publish(next: Partial<DictionarySnapshot>): void {
  snapshot = { ...snapshot, ...next };
  for (const listener of [...listeners]) listener();
}

export function getDictionarySnapshot(): DictionarySnapshot {
  return snapshot;
}

export function subscribeDictionary(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Whether a row waits for the person: a suggestion, or an agent's row not yet kept. */
export function isWaiting(entry: DictionaryEntryView): boolean {
  return (
    entry.status === 'suggested' ||
    (entry.status === 'active' && entry.source === 'agent' && !entry.reviewed)
  );
}

/** Read every page and the settings now. Resolves when this read has been published or dropped. */
export async function refreshDictionary(): Promise<void> {
  const issued = ++generation;
  const cache = pageCache;
  if (snapshot.load === 'unloaded' || snapshot.load === 'failed') publish({ load: 'loading' });
  const headers = await userActionHeaders();
  const settingsRead = settle<DictionarySettings>(
    getDictionarySettings({ headers }) as Promise<RawResult>
  );
  const entries: DictionaryEntryView[] = [];
  let failure: DictionaryFailure | null = null;
  let changed = false;
  let pagesRead = 0;
  let cursor: string | null = null;
  // At most 500 rows a page; 5,000 active rows a tier and the suggestions fit in a few dozen.
  for (; pagesRead < 64; pagesRead++) {
    const read = await readPage(cache, 'all', cursor, headers);
    // A newer read has started (a write's, a focus): this one would be dropped, so stop asking.
    if (issued !== generation) return;
    if (!read.ok) {
      failure = read.error;
      break;
    }
    changed ||= read.changed;
    entries.push(...read.page.entries);
    cursor = read.page.next_cursor ?? null;
    if (!cursor) break;
  }
  const settings = await settingsRead;
  if (issued !== generation) return;
  const nextSettings = settings.ok ? settings.data : snapshot.settings;
  const sameSettings = JSON.stringify(nextSettings) === JSON.stringify(snapshot.settings);
  if (failure) {
    const absent = failure.status === 404 || failure.status === 405;
    publish({
      load: snapshot.load === 'loaded' ? 'loaded' : 'failed',
      error: failure.message,
      absent,
      ...(settings.ok && !sameSettings ? { settings: settings.data } : {}),
    });
    return;
  }
  // Every page answered unchanged (and as many pages as before): keep the very same rows, so a
  // quiet minute renders nothing.
  const unchanged =
    !changed && snapshot.load === 'loaded' && entries.length === snapshot.entries.length;
  if (unchanged && sameSettings && snapshot.error === null && !snapshot.absent) return;
  publish({
    load: 'loaded',
    entries: unchanged ? snapshot.entries : entries,
    error: null,
    absent: false,
    settings: sameSettings ? snapshot.settings : nextSettings,
  });
  noteWaitingFromList(entries);
}

/** Settings-only reads issued; a read that lands after a newer one started is dropped. */
let settingsGeneration = 0;

/**
 * Read the settings answer alone, for a surface that shows a setting but not the rows (the
 * Speech models section in Settings). It publishes into the same snapshot the page reads, so the
 * two views show one value. Dropped when a newer settings read, or a full read, started after it.
 */
export async function refreshDictionarySettings(): Promise<void> {
  const issued = ++settingsGeneration;
  const listingAtIssue = generation;
  const read = await settle<DictionarySettings>(
    getDictionarySettings({ headers: await userActionHeaders() }) as Promise<RawResult>
  );
  if (issued !== settingsGeneration || listingAtIssue !== generation || !read.ok) return;
  if (JSON.stringify(read.data) === JSON.stringify(snapshot.settings)) return;
  publish({ settings: read.data });
}

/** The settings answer, read once when nothing has read it yet (the context menu's door). */
export async function loadDictionarySettings(): Promise<DictionarySettings | null> {
  if (snapshot.settings) return snapshot.settings;
  const settings = await settle<DictionarySettings>(
    getDictionarySettings({ headers: await userActionHeaders() }) as Promise<RawResult>
  );
  if (settings.ok) publish({ settings: settings.data });
  return settings.ok ? settings.data : null;
}

// ── writes ───────────────────────────────────────────────────────────────────

/** What a write's own answer says about one row: the row as the daemon now holds it, or its id gone. */
type Answered = { row: DictionaryEntryView } | { removed: string };

/** The listing read again after the last write that answered its row; tests wait on it. */
let rereading: Promise<void> = Promise.resolve();

/**
 * Put the row a write answered into the listing at once: the daemon's own answer for that row
 * (confirmed state, never intent), newest change first as the listing orders it.
 */
function applyAnswered(answered: Answered): void {
  if (snapshot.load !== 'loaded') return;
  const id = 'row' in answered ? answered.row.id : answered.removed;
  const rest = snapshot.entries.filter((entry) => entry.id !== id);
  const entries = 'row' in answered ? [answered.row, ...rest] : rest;
  publish({ entries });
  noteWaitingFromList(entries);
}

/**
 * Run a write, then read the listing again (confirmed state). A write whose answer carries the
 * row it changed (an add, an edit, a tier change, a delete) shows that row at once and answers
 * then, while the listing is read again behind it: at the 5,000 row cap the listing is ten pages
 * read one after another, about 220 ms in a development build, which used to sit between the
 * click and the row. Every other write, and every refusal, answers after the listing was read.
 */
async function write<T>(
  call: (headers: Record<string, string>) => Promise<unknown>,
  answered?: (data: T) => Answered | null
): Promise<Outcome<T>> {
  const outcome = await settle<T>(call(await userActionHeaders()) as Promise<RawResult>);
  const change = outcome.ok && answered ? answered(outcome.data) : null;
  if (change) {
    applyAnswered(change);
    rereading = refreshDictionary().catch(() => undefined);
    return outcome;
  }
  await refreshDictionary();
  return outcome;
}

/** The row an answer carries (`{entry}`), checked, since a body can be missing. */
function answeredRow(data: { entry?: DictionaryEntryView | null } | null | undefined) {
  return data?.entry ? { row: data.entry } : null;
}

export function addTerm(input: DictionaryAddRequest): Promise<Outcome<DictionaryEntryAnswer>> {
  return write<DictionaryEntryAnswer>(
    (headers) => addDictionaryEntry({ body: input, headers }),
    answeredRow
  );
}

export function editTerm(
  id: string,
  rev: number,
  patch: DictionaryEditRequest
): Promise<Outcome<{ entry: DictionaryEntryView }>> {
  return write<{ entry: DictionaryEntryView }>(
    (headers) =>
      editDictionaryEntry({
        path: { id },
        body: patch,
        headers: { ...headers, 'If-Match': String(rev) },
      }),
    answeredRow
  );
}

export function setTermTier(
  id: string,
  tier: DictionaryTier
): Promise<Outcome<{ entry: DictionaryEntryView }>> {
  return write<{ entry: DictionaryEntryView }>(
    (headers) => changeDictionaryEntryTier({ path: { id }, body: { tier }, headers }),
    answeredRow
  );
}

export function deleteTerm(id: string): Promise<Outcome<unknown>> {
  return write(
    (headers) => deleteDictionaryEntry({ path: { id }, headers }),
    () => ({ removed: id })
  );
}

/** Undo a tool-added row only while that exact unreviewed revision still exists. */
export function undoAgentAdd(id: string, expectedRev: number): Promise<Outcome<unknown>> {
  return write(
    (headers) =>
      undoAgentDictionaryAdd({ path: { id }, body: { expected_rev: expectedRev }, headers }),
    () => ({ removed: id })
  );
}

/** Delete listed rows, 5,000 ids a request; answers how many were deleted. */
export async function deleteTerms(ids: readonly string[]): Promise<Outcome<number>> {
  const headers = await userActionHeaders();
  let deleted = 0;
  let outcome: Outcome<number> | null = null;
  for (let at = 0; at < ids.length; at += MAX_IDS_PER_REQUEST) {
    const part = await settle<{ deleted: number }>(
      deleteDictionaryEntries({
        body: { ids: ids.slice(at, at + MAX_IDS_PER_REQUEST) },
        headers,
      }) as Promise<RawResult>
    );
    if (!part.ok) {
      outcome = part;
      break;
    }
    deleted += part.data.deleted;
  }
  await refreshDictionary();
  return outcome ?? { ok: true, status: 200, data: deleted };
}

export function setAgentWrites(
  agentWrites: DictionaryAgentWrites
): Promise<Outcome<DictionarySettings>> {
  return write((headers) =>
    changeDictionarySettings({ body: { agent_writes: agentWrites }, headers })
  );
}

export function setSuggestFromChats(suggest: boolean): Promise<Outcome<DictionarySettings>> {
  return write((headers) => activateDictionary({ body: { suggest_from_chats: suggest }, headers }));
}

/** `learn_from_chats`: the chat extractor's switch (PR-24), through `POST /dictionary/settings`. */
export function setLearnFromChats(learn: boolean): Promise<Outcome<DictionarySettings>> {
  return write((headers) =>
    changeDictionarySettings({ body: { learn_from_chats: learn }, headers })
  );
}

/**
 * The person's answer **Keep suggesting** to the offer to turn suggestions off
 * (`suggestions.offer_turning_off`), which quiets the offer for 20 more decisions.
 */
export function keepSuggesting(): Promise<Outcome<DictionarySettings>> {
  return write((headers) => changeDictionarySettings({ body: { keep_suggesting: true }, headers }));
}

/**
 * Whether **Suggest words from my messages** may be switched now, and why not. Turning it on is
 * a raise (`authority.raise_settings`); turning it off always works. The reason is the daemon's
 * (MSG-DICTIONARY-HOST-SETTINGS on a keyless `serve` daemon), its `<option>` filled for this
 * switch, and shows only while the switch is off and cannot be turned on.
 */
export function learnFromChatsGate(settings: DictionarySettings | null): {
  disabled: boolean;
  reason: string | null;
} {
  const authority = settings?.authority ?? null;
  if (!settings || !authority) return { disabled: true, reason: null };
  if (settings.learn_from_chats || authority.raise_settings.allowed) {
    return { disabled: false, reason: null };
  }
  const reason = authority.raise_settings.reason?.trim();
  return {
    disabled: true,
    reason: reason ? fillMessage(reason, { option: '--learn-from-chats on' }) : null,
  };
}

export function acceptTerms(ids: readonly string[]): Promise<Outcome<{ accepted: number }>> {
  return write((headers) => acceptDictionarySuggestions({ body: { ids: [...ids] }, headers }));
}

export function dismissTerms(ids: readonly string[]): Promise<Outcome<{ dismissed: number }>> {
  return write((headers) => dismissDictionarySuggestions({ body: { ids: [...ids] }, headers }));
}

export function keepTerms(ids: readonly string[]): Promise<Outcome<{ kept: number }>> {
  return write((headers) => keepDictionaryEntries({ body: { ids: [...ids] }, headers }));
}

/** `POST /dictionary/import` with `dry_run: true`: what an import would do, writing nothing. */
export async function planImport(
  input: Omit<DictionaryImportRequest, 'dry_run'>
): Promise<Outcome<DictionaryImportAnswer>> {
  return settle(
    importDictionary({
      body: { ...input, dry_run: true },
      headers: await userActionHeaders(),
    }) as Promise<RawResult>
  );
}

export function applyImport(
  input: Omit<DictionaryImportRequest, 'dry_run'>
): Promise<Outcome<DictionaryImportAnswer>> {
  return write((headers) => importDictionary({ body: { ...input, dry_run: false }, headers }));
}

/** The export file's text: public rows, and private rows only when asked for with standing. */
export async function exportTerms(includePrivate: boolean): Promise<Outcome<string>> {
  return settle(
    exportDictionary({
      query: { include_private: includePrivate },
      headers: await userActionHeaders(),
      parseAs: 'text',
    }) as Promise<RawResult>
  );
}

// ── the waiting count, for the sidebar ───────────────────────────────────────

let waiting = 0;
/** Unreviewed agent rows the last full listing held; the sidebar's own read counts suggestions only. */
let waitingAgentRows = 0;
const waitingListeners = new Set<() => void>();
let waitingCache = new Map<string, CachedPage>();
let waitingGeneration = 0;

function publishWaiting(next: number): void {
  if (next === waiting) return;
  waiting = next;
  for (const listener of [...waitingListeners]) listener();
}

function noteWaitingFromList(entries: readonly DictionaryEntryView[]): void {
  const suggested = entries.filter((entry) => entry.status === 'suggested').length;
  waitingAgentRows = entries.filter(
    (entry) => isWaiting(entry) && entry.status === 'active'
  ).length;
  publishWaiting(suggested + waitingAgentRows);
}

export function getWaitingCount(): number {
  return waiting;
}

function subscribeWaiting(listener: () => void): () => void {
  waitingListeners.add(listener);
  return () => waitingListeners.delete(listener);
}

/**
 * The sidebar's read: `GET /dictionary/entries?status=suggested` with
 * `If-None-Match`. It counts the first page (500 rows at most, more than any
 * badge shows), plus the unreviewed agent rows the page last listed.
 */
export async function refreshWaitingCount(): Promise<void> {
  const issued = ++waitingGeneration;
  const cache = waitingCache;
  const read = await readPage(cache, 'suggested', null, await userActionHeaders());
  if (issued !== waitingGeneration || !read.ok) return;
  publishWaiting(read.page.entries.length + waitingAgentRows);
}

// ── live refresh ─────────────────────────────────────────────────────────────

function visible(): boolean {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden';
}

/**
 * Read with `read` on window focus, when the window becomes visible, and every
 * `DICTIONARY_REFRESH_MS` while it is visible. Answers the disposer.
 */
export function installLiveRefresh(read: () => void): () => void {
  const onFocus = () => read();
  const onVisibility = () => {
    if (visible()) read();
  };
  const timer = setInterval(() => {
    if (visible()) read();
  }, DICTIONARY_REFRESH_MS);
  window.addEventListener('focus', onFocus);
  document.addEventListener('visibilitychange', onVisibility);
  return () => {
    clearInterval(timer);
    window.removeEventListener('focus', onFocus);
    document.removeEventListener('visibilitychange', onVisibility);
  };
}

// ── hooks ────────────────────────────────────────────────────────────────────

export interface DictionaryHook extends DictionarySnapshot {
  refresh: () => Promise<void>;
  add: typeof addTerm;
  edit: typeof editTerm;
  setTier: typeof setTermTier;
  remove: typeof deleteTerm;
  removeMany: typeof deleteTerms;
  accept: typeof acceptTerms;
  dismiss: typeof dismissTerms;
  keep: typeof keepTerms;
  setAgentWrites: typeof setAgentWrites;
  setSuggestFromChats: typeof setSuggestFromChats;
  setLearnFromChats: typeof setLearnFromChats;
  keepSuggesting: typeof keepSuggesting;
  importPlan: typeof planImport;
  importApply: typeof applyImport;
  exportFile: typeof exportTerms;
}

/** The page's data: read on mount, then live while mounted. */
export function useDictionary(): DictionaryHook {
  const current = useSyncExternalStore(subscribeDictionary, getDictionarySnapshot);
  useEffect(() => {
    void refreshDictionary();
    return installLiveRefresh(() => void refreshDictionary());
  }, []);
  return {
    ...current,
    refresh: refreshDictionary,
    add: addTerm,
    edit: editTerm,
    setTier: setTermTier,
    remove: deleteTerm,
    removeMany: deleteTerms,
    accept: acceptTerms,
    dismiss: dismissTerms,
    keep: keepTerms,
    setAgentWrites,
    setSuggestFromChats,
    setLearnFromChats,
    keepSuggesting,
    importPlan: planImport,
    importApply: applyImport,
    exportFile: exportTerms,
  };
}

function getDictionarySettingsSnapshot(): DictionarySettings | null {
  return snapshot.settings;
}

/**
 * The settings answer alone, from the page's own store: read on mount and live while mounted
 * (window focus, visibility, every 60 s), and moved at once by a write from either surface.
 * Reads only while `enabled`.
 */
export function useDictionarySettings(enabled: boolean): DictionarySettings | null {
  const settings = useSyncExternalStore(subscribeDictionary, getDictionarySettingsSnapshot);
  useEffect(() => {
    if (!enabled) return;
    void refreshDictionarySettings();
    return installLiveRefresh(() => void refreshDictionarySettings());
  }, [enabled]);
  return enabled ? settings : null;
}

/**
 * How many rows wait for the person, for the sidebar's count and dot. Reads only
 * while `enabled` (the dictionary is allowed and this is the desktop), on mount,
 * on window focus and every 60 s while visible.
 */
export function useDictionaryWaitingCount(enabled: boolean): number {
  const count = useSyncExternalStore(subscribeWaiting, getWaitingCount);
  useEffect(() => {
    if (!enabled) return;
    void refreshWaitingCount();
    return installLiveRefresh(() => void refreshWaitingCount());
  }, [enabled]);
  return enabled ? count : 0;
}

// ── tests ────────────────────────────────────────────────────────────────────

/** For tests: start over. A read still in flight from before is dropped when it lands. */
export function resetDictionaryStoreForTests(): void {
  snapshot = INITIAL;
  listeners.clear();
  // New maps, so a read still in flight from before writes into the old ones.
  pageCache = new Map();
  generation += 1;
  settingsGeneration += 1;
  rereading = Promise.resolve();
  waiting = 0;
  waitingAgentRows = 0;
  waitingListeners.clear();
  waitingCache = new Map();
  waitingGeneration += 1;
}

/** For tests: the listing read again after the last write that answered its row. */
export function dictionaryRereadForTests(): Promise<void> {
  return rereading;
}
