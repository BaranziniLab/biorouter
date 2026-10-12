import { listSessions, type Session } from '../api';
import { userActionHeaders } from './userAction';
import { laterActivity, subscribeSessionNameChanges } from './sessionNameSync';
import {
  settleRowsReadDuringFetch,
  subscribeSessionRowChanges,
  type SessionRowFacts,
} from './sessionRowSync';

let cachedSessions: Session[] | null = null;
let inFlightRequest: Promise<Session[]> | null = null;
// The newest-rows read (`refreshSessionListHead`). It never overlaps a full
// read: a head read waits for one in flight, and a full read waits for a head
// read, because both settle against the same "published while in flight" maps
// below and a read that cleared them under the other would lose a rename or a
// raise the other still has to apply.
let inFlightHead: Promise<SessionListHead> | null = null;
// When a full read last landed (epoch ms). A head read leaves the rows after
// its page as the cache held them, so this is how long they have gone unchecked.
let lastFullReadAt = 0;
let staleReadTimer: ReturnType<typeof setTimeout> | null = null;
// BR-71: the `include_subagents` query key is part of the cache identity, or a
// toggle serves the stale list and never refetches.
let cachedIncludeSubagents = false;
// Bumped whenever a request in flight is orphaned (a flag change, a cache
// clear). Nothing can cancel an in-flight fetch, so each request captures the
// generation it was issued under and, on settling, checks it is still the
// current one before touching any module state. Without this an orphan writes
// its answer into the cache — emitting the wrong-shaped list to every
// subscriber — and its `.finally` nulls the in-flight slot its successor owns.
let requestGeneration = 0;
const listeners = new Set<() => void>();

function emitChange(): void {
  for (const listener of listeners) listener();
}

/**
 * Names the name channel published WHILE a list request was in flight.
 *
 * A list response describes the moment it was ISSUED, so a rename that landed
 * after that is the LATER fact — but `refreshSessionList` replaces the whole
 * array, which would undo it. That is the snap-back `sessionNameSync`'s header
 * describes, and until now it only cost Home recents and See-all a wrong name
 * until their next refresh. It costs more now: the tab strip reconciles its
 * titles against this cache (`ChatGroupsShell.useTabTitlesFromSessionList`), so
 * a clobbered name would be written onto a tab AND persisted there.
 *
 * The window is not hypothetical. A brand-new chat's first turn opens both
 * halves of it: `refreshSessionBinding` finds the chat missing from a fetched
 * list and calls `notifySessionListChanged` (→ a full `GET /sessions`), and
 * ~800 ms later the auto-name poll announces the generated name. On a machine
 * with thousands of chats the list is easily the slower of the two.
 *
 * Recorded only while a request is outstanding, and cleared by the request that
 * consumes them, so the map cannot grow. A saved rename's `activityAt` is kept
 * with its name, because a list issued before the rename also carries the old
 * place in the order (R-30).
 */
const namesPublishedDuringFetch = new Map<
  string,
  { name: string; userSetName: boolean; activityAt?: string }
>();

/**
 * Put a renamed chat where its activity now puts it (R-30: a saved rename is
 * activity). The list is in the daemon's order, `updated_at` newest first, and
 * History keeps that order inside each date bucket, so the row is lifted to
 * just above the first row that is not newer than it, with its `updated_at`
 * raised to the rename. A row the list does not hold is left to the re-read.
 */
export function placeRenamedSession(
  sessions: Session[],
  sessionId: string,
  activityAt: string
): Session[] {
  const index = sessions.findIndex((session) => session.id === sessionId);
  if (index === -1) return sessions;
  const row = sessions[index];
  const updatedAt = laterActivity(row.updated_at, activityAt);
  const at = Date.parse(updatedAt);
  if (Number.isNaN(at)) return sessions;
  const rest = sessions.filter((_, i) => i !== index);
  const before = rest.findIndex((session) => !(Date.parse(session.updated_at) > at));
  const insertAt = before === -1 ? rest.length : before;
  return [...rest.slice(0, insertAt), { ...row, updated_at: updatedAt }, ...rest.slice(insertAt)];
}

// A session rename rides the name channel, not the list channel — but the
// See-all view and Home recents read THIS cache, so patch the cached name when
// one arrives. Without this a rename made in the tab pill or the sidebar never
// reached those two surfaces until they remounted, and the same session showed
// two different names in two panels at once.
//
// A rename the daemon has SAVED (`activityAt`) also moves the chat (R-30):
// History and Home recents show it at the top at once, with no request. A chat
// the cache does not hold yet (a list read before the chat had its first
// message) is brought in by re-reading the list instead.
subscribeSessionNameChanges(({ sessionId, name, userSetName, activityAt }) => {
  if (inFlightRequest || inFlightHead) {
    namesPublishedDuringFetch.set(sessionId, { name, userSetName, activityAt });
  }
  if (!cachedSessions) return;
  const idx = cachedSessions.findIndex((s) => s.id === sessionId);
  if (idx === -1) {
    if (activityAt) void refreshSessionList().catch(() => undefined);
    return;
  }
  let next = cachedSessions.slice();
  next[idx] = { ...next[idx], name, user_set_name: userSetName };
  if (activityAt) next = placeRenamedSession(next, sessionId, activityAt);
  cachedSessions = next;
  emitChange();
});

/**
 * Row reads (`sessionRowSync`) delivered WHILE a list request was in flight,
 * settled against that request's answer by `settleRowsReadDuringFetch`.
 *
 * ⚠ This used to be "not re-applied — a list that snaps a row back to private
 * is the safe miss". The snap-back runs in BOTH directions, and the other one
 * is not safe: a list issued before a turn raised a chat, landing after the
 * raise was patched in, draws the chat public again. Recorded and cleared
 * exactly as {@link namesPublishedDuringFetch} is, so it cannot grow.
 */
const rowsReadDuringFetch = new Map<string, SessionRowFacts>();

// A row whose classification changed in place — a declassification, or a raise
// a chat store announced, in this window or another (`sessionRowSync`).
// History's badge, its row menu ("Make this chat public" is offered on private
// rows only), Home recents and the tab strip's cached tiers all read this cache,
// so the entry is patched where it sits. Nothing is re-sorted: the daemon no
// longer moves `updated_at` for a classification change, so the row's place in
// the list is still right.
subscribeSessionRowChanges((facts) => {
  const { sessionId, privacy_tier, privacy_reason } = facts;
  if (inFlightRequest || inFlightHead) rowsReadDuringFetch.set(sessionId, facts);
  if (!cachedSessions) return;
  const idx = cachedSessions.findIndex((s) => s.id === sessionId);
  if (idx === -1) return;
  const current = cachedSessions[idx];
  if (
    current.privacy_tier === privacy_tier &&
    (current.privacy_reason ?? null) === privacy_reason
  ) {
    return;
  }
  const next = cachedSessions.slice();
  next[idx] = { ...current, privacy_tier, privacy_reason };
  cachedSessions = next;
  emitChange();
});

// ── Cross-window "the set of sessions changed" signal ──────────────────────
// A sibling to sessionNameSync's name channel, for list MEMBERSHIP: a session
// created, diverged, deleted or imported. Every list surface — sidebar Recents,
// the See-all view, Home recents — subscribes and re-reads, so a branch created
// in one window appears in ALL of them without a manual refresh. (A rename is
// NOT membership; it uses the name channel above.)
type ListListener = () => void;
const listChangeListeners = new Set<ListListener>();
let listChannel: BroadcastChannel | null = null;

/**
 * What changed, when the change is more specific than "something did".
 *
 * M11: a plain "re-read now" nudge is enough to ADD a session and not enough to
 * remove one. The sidebar Recents merges each re-read into what it already holds
 * (`appendSessionPage` appends and replaces by id, and has no removal branch),
 * so a deleted chat survived every refresh and cleared only on a renderer
 * reload. Removals are therefore announced BY ID rather than inferred from a
 * refetch — `loadPage(true)` re-reads only the first page, and an entry can drop
 * out of that window because other chats were touched rather than because it was
 * deleted, so inference would evict live chats.
 */
export interface SessionListChange {
  /** The id of a session that no longer exists. */
  removed?: string;
}

type RemovalListener = (sessionId: string) => void;
const removalListeners = new Set<RemovalListener>();

function fanOutRemoval(sessionId: string | undefined): void {
  if (!sessionId) return;
  for (const l of removalListeners) l(sessionId);
}

function getListChannel(): BroadcastChannel | null {
  if (listChannel) return listChannel;
  if (typeof BroadcastChannel === 'undefined') return null;
  listChannel = new BroadcastChannel('biorouter:session-list');
  listChannel.onmessage = (event: MessageEvent) => {
    // Another window changed the set. Refresh this window's own cache (so its
    // See-all/Home update) and fan out to hooks that fetch their own way.
    fanOutRemoval((event.data as SessionListChange | undefined)?.removed);
    void refreshSessionList().catch(() => undefined);
    for (const l of listChangeListeners) l();
  };
  return listChannel;
}

/**
 * Subscribe to session REMOVALS specifically, by id. For a surface that keeps
 * its own merged list and cannot discover a deletion by re-reading — see
 * {@link SessionListChange}.
 */
export function subscribeSessionRemoved(listener: RemovalListener): () => void {
  getListChannel();
  removalListeners.add(listener);
  return () => {
    removalListeners.delete(listener);
  };
}

/**
 * Subscribe to list-membership changes (this window and every sibling window).
 * For surfaces that keep their own list (the sidebar Recents uses a different
 * endpoint than this cache) and just need a "re-read now" nudge.
 */
export function subscribeSessionListChanges(listener: ListListener): () => void {
  getListChannel();
  listChangeListeners.add(listener);
  return () => {
    listChangeListeners.delete(listener);
  };
}

/**
 * Announce that the set of sessions changed — call after create, diverge,
 * delete or import. Refreshes this window's cache immediately and notifies every
 * other window to do the same.
 */
export function notifySessionListChanged(change: SessionListChange = {}): void {
  // Removals go out FIRST and synchronously. The nudge below is debounced by
  // most subscribers, so a listener that splices by id must not be racing a
  // re-read that would merge the doomed row straight back in.
  fanOutRemoval(change.removed);
  void refreshSessionList().catch(() => undefined);
  for (const l of listChangeListeners) l();
  getListChannel()?.postMessage({ at: Date.now(), ...change });
}

/** Re-apply the renames that outran this response. See {@link namesPublishedDuringFetch}. */
function applyNamesPublishedDuringFetch(sessions: Session[]): Session[] {
  if (namesPublishedDuringFetch.size === 0) return sessions;
  let next = sessions.map((session) => {
    const later = namesPublishedDuringFetch.get(session.id);
    return later ? { ...session, name: later.name, user_set_name: later.userSetName } : session;
  });
  for (const [sessionId, { activityAt }] of namesPublishedDuringFetch) {
    if (activityAt) next = placeRenamedSession(next, sessionId, activityAt);
  }
  namesPublishedDuringFetch.clear();
  return next;
}

export function getCachedSessionList(): Session[] | null {
  return cachedSessions;
}

export function updateCachedSessionList(
  update: Session[] | ((sessions: Session[]) => Session[])
): void {
  const current = cachedSessions ?? [];
  cachedSessions = typeof update === 'function' ? update(current) : update;
  emitChange();
}

export function subscribeSessionList(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

// ⚠ `includeSubagents?: boolean`, NOT `= false`. This module has TWO consumers
// (History and Home's `SessionsInsights`) and only one of them has an opinion;
// a keyless call must mean "whatever is cached", not "false", or Home's every
// render would invalidate History's toggle and silently drop the children.
export async function refreshSessionList(includeSubagents?: boolean): Promise<Session[]> {
  // A flag change invalidates both the in-flight request and the cache: the
  // dedupe below is keyed only on "a request is running", so without this a
  // toggle during an in-flight fetch would resolve to the OTHER flag's result.
  if (includeSubagents !== undefined && includeSubagents !== cachedIncludeSubagents) {
    cachedIncludeSubagents = includeSubagents;
    cachedSessions = null;
    inFlightRequest = null;
    inFlightHead = null;
    requestGeneration += 1;
  }
  if (inFlightRequest) return inFlightRequest;

  const generation = requestGeneration;
  // With the user's proof: since issue #56's QA sweep (2026-09-10) a listing
  // omits every private chat from a caller without it, as the singular read
  // refuses one — and this app is the person at the keyboard.
  // `cachedIncludeSubagents`, not the parameter: a keyless call must send the
  // identity the cache is holding, not `undefined`. Read NOW, before the
  // proof's async hop: a flag change in that gap orphans this request, and
  // an orphan must still ask for the list it was issued for.
  const issuedFor = cachedIncludeSubagents;
  const send = () => {
    // This request's answer supersedes every name it is about to carry — except
    // the ones announced from HERE on. See {@link namesPublishedDuringFetch}.
    namesPublishedDuringFetch.clear();
    rowsReadDuringFetch.clear();
    return userActionHeaders().then((headers) =>
      listSessions<true>({
        throwOnError: true,
        headers,
        query: { include_subagents: issuedFor },
      })
    );
  };
  // Behind a head read in flight, never beside it. See {@link inFlightHead}.
  const behindHead = inFlightHead;
  inFlightRequest = (behindHead ? behindHead.then(send, send) : send())
    .then((response) => {
      // Superseded while in flight: hand the answer back to whoever awaited
      // this exact call, but publish nothing — the cache and its subscribers
      // belong to the request that replaced it.
      if (generation !== requestGeneration) return response.data.sessions;
      cachedSessions = settleRowsReadDuringFetch(
        applyNamesPublishedDuringFetch(response.data.sessions),
        rowsReadDuringFetch,
        true
      );
      lastFullReadAt = Date.now();
      emitChange();
      return cachedSessions;
    })
    .finally(() => {
      if (generation === requestGeneration) inFlightRequest = null;
    });

  return inFlightRequest;
}

/**
 * How many of the newest chats a head read asks for. History shows 16 rows and
 * adds 20 as the reader scrolls, so this covers the first three screens; the
 * rows after it come from the cache.
 */
export const SESSION_LIST_HEAD_SIZE = 40;
/**
 * How long the rows a head read left as the cache held them may go unchecked
 * before the next one also schedules a quiet full read. A full read is what
 * notices a chat deleted from the terminal and a roster changed by housekeeping,
 * neither of which moves a chat into the newest page.
 */
export const SESSION_LIST_STALE_AFTER_MS = 60_000;
const STALE_READ_DELAY_MS = 1_500;

/** What {@link refreshSessionListHead} answers. */
export interface SessionListHead {
  /** The list to show now. */
  sessions: Session[];
  /**
   * `true` when `sessions` is only the newest page: the cache was cold, so the
   * page could not become it, and the full read that fills it has been started.
   * Its arrival reaches the caller through {@link subscribeSessionList}.
   */
  partial: boolean;
}

/**
 * The cached list with a fresh page of its newest rows laid over it, in the
 * daemon's order.
 *
 * Every row the page holds replaces the cached copy of that chat. A cached row
 * that sorts BEFORE the page's last row but is not in the page is dropped: the
 * page is the daemon's whole answer for that stretch of the ordering, so such a
 * chat is gone (deleted) or no longer listed to this caller. Rows after the page
 * stay as the cache held them.
 *
 * A row whose `updated_at` cannot be read is kept: a list that shows a chat it
 * should not is repaired by the next full read, and one that hides a chat is
 * not. So is a row in `keep`, the chats this window was told something about
 * while the page was in flight: a saved rename raised its cached `updated_at`
 * past a page issued before it, and that page not holding the chat says nothing
 * about the chat.
 */
export function mergeSessionListHead(
  cached: Session[],
  head: Session[],
  keep: ReadonlySet<string> = new Set()
): Session[] {
  const last = head[head.length - 1];
  if (!last) return head;
  const headIds = new Set(head.map((session) => session.id));
  const boundary = Date.parse(last.updated_at);
  const tail = cached.filter((session) => {
    if (headIds.has(session.id)) return false;
    if (keep.has(session.id)) return true;
    const at = Date.parse(session.updated_at);
    if (Number.isNaN(at) || Number.isNaN(boundary)) return true;
    // The daemon's order is `updated_at` newest first, then `id` ascending.
    return at < boundary || (at === boundary && session.id > last.id);
  });
  return head.concat(tail);
}

function scheduleStaleRead(): void {
  if (staleReadTimer !== null) return;
  if (Date.now() - lastFullReadAt < SESSION_LIST_STALE_AFTER_MS) return;
  staleReadTimer = setTimeout(() => {
    staleReadTimer = null;
    void refreshSessionList().catch(() => undefined);
  }, STALE_READ_DELAY_MS);
}

/** Settle a head page against what was published while it was in flight. */
function settleHead(page: Session[], more: boolean): SessionListHead {
  const settle = (rows: Session[]) =>
    settleRowsReadDuringFetch(applyNamesPublishedDuringFetch(rows), rowsReadDuringFetch, true);
  if (!more) {
    // The page IS the list: a short store, or a daemon that ignores `limit`.
    cachedSessions = settle(page);
    lastFullReadAt = Date.now();
    emitChange();
    return { sessions: cachedSessions, partial: false };
  }
  if (cachedSessions === null) {
    // Cold (the first sidebar read has not landed, or the cache was cleared). A
    // page of the newest rows cannot be the cache that Home, the tab strip and
    // the sidebar read, so it is handed back alone and the full read is started.
    const rows = settle(page);
    void refreshSessionList().catch(() => undefined);
    return { sessions: rows, partial: true };
  }
  cachedSessions = settle(
    mergeSessionListHead(cachedSessions, page, new Set(namesPublishedDuringFetch.keys()))
  );
  emitChange();
  scheduleStaleRead();
  return { sessions: cachedSessions, partial: false };
}

/**
 * Read the newest rows of the listing and lay them over the cache: the fast way
 * to open History on a list that is current at the top (V1-R3-2).
 *
 * The full read is one request for every chat on the machine (5,542 on the
 * store this was measured on: 6 MB, 0.5 to 0.8 s before the rows could be
 * drawn), and almost all of it is old chats that nothing has touched. A chat
 * only moves in the order when something happens in it (R-30), and anything that
 * happens in it puts it among the newest, so the newest page is where a change
 * shows up. The rows after it keep what the cache held, which is the last full
 * read plus every rename, privacy change and removal this window was told of.
 *
 * Not the head of the identity this cache holds? A list with sub-agent runs
 * nests each under its parent, which can sit anywhere in the order, so that and
 * any change of identity go to {@link refreshSessionList}. A full read already
 * in flight is waited for rather than raced.
 */
export async function refreshSessionListHead(includeSubagents?: boolean): Promise<SessionListHead> {
  const wanted = includeSubagents ?? cachedIncludeSubagents;
  if (wanted || wanted !== cachedIncludeSubagents) {
    return { sessions: await refreshSessionList(includeSubagents), partial: false };
  }
  if (inFlightRequest) return { sessions: await inFlightRequest, partial: false };
  if (inFlightHead) return inFlightHead;

  const generation = requestGeneration;
  namesPublishedDuringFetch.clear();
  rowsReadDuringFetch.clear();
  const request: Promise<SessionListHead> = userActionHeaders()
    .then((headers) =>
      listSessions<true>({
        throwOnError: true,
        headers,
        query: { include_subagents: false, limit: SESSION_LIST_HEAD_SIZE },
      })
    )
    .then((response) => {
      const page = response.data.sessions;
      const more = response.data.has_more === true;
      // Superseded while in flight: hand the page back, publish nothing.
      if (generation !== requestGeneration) return { sessions: page, partial: more };
      return settleHead(page, more);
    })
    .finally(() => {
      if (inFlightHead === request) inFlightHead = null;
    });
  inFlightHead = request;
  return request;
}

export function preloadSessionList(): void {
  if (cachedSessions !== null) return;
  void refreshSessionList().catch(() => undefined);
}

export function clearSessionListCache(): void {
  cachedSessions = null;
  inFlightRequest = null;
  inFlightHead = null;
  lastFullReadAt = 0;
  if (staleReadTimer !== null) clearTimeout(staleReadTimer);
  staleReadTimer = null;
  cachedIncludeSubagents = false;
  // Same orphaning as a flag change: a request issued before the clear must not
  // repopulate the cache we just emptied.
  requestGeneration += 1;
  emitChange();
}
