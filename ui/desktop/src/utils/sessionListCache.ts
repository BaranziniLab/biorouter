import { listSessions, type Session } from '../api';
import { userActionHeaders } from './userAction';
import { subscribeSessionNameChanges } from './sessionNameSync';

let cachedSessions: Session[] | null = null;
let inFlightRequest: Promise<Session[]> | null = null;
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
 * consumes them, so the map cannot grow.
 */
const namesPublishedDuringFetch = new Map<string, { name: string; userSetName: boolean }>();

// A session rename rides the name channel, not the list channel — but the
// See-all view and Home recents read THIS cache, so patch the cached name when
// one arrives. Without this a rename made in the tab pill or the sidebar never
// reached those two surfaces until they remounted, and the same session showed
// two different names in two panels at once.
subscribeSessionNameChanges(({ sessionId, name, userSetName }) => {
  if (inFlightRequest) namesPublishedDuringFetch.set(sessionId, { name, userSetName });
  if (!cachedSessions) return;
  const idx = cachedSessions.findIndex((s) => s.id === sessionId);
  if (idx === -1) return;
  const next = cachedSessions.slice();
  next[idx] = { ...next[idx], name, user_set_name: userSetName };
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
  const next = sessions.map((session) => {
    const later = namesPublishedDuringFetch.get(session.id);
    return later ? { ...session, name: later.name, user_set_name: later.userSetName } : session;
  });
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
  // This request's answer supersedes every name it is about to carry — except
  // the ones announced from HERE on. See {@link namesPublishedDuringFetch}.
  namesPublishedDuringFetch.clear();
  inFlightRequest = userActionHeaders()
    .then((headers) =>
      listSessions<true>({
        throwOnError: true,
        headers,
        query: { include_subagents: issuedFor },
      })
    )
    .then((response) => {
      // Superseded while in flight: hand the answer back to whoever awaited
      // this exact call, but publish nothing — the cache and its subscribers
      // belong to the request that replaced it.
      if (generation !== requestGeneration) return response.data.sessions;
      cachedSessions = applyNamesPublishedDuringFetch(response.data.sessions);
      emitChange();
      return cachedSessions;
    })
    .finally(() => {
      if (generation === requestGeneration) inFlightRequest = null;
    });

  return inFlightRequest;
}

export function preloadSessionList(): void {
  if (cachedSessions !== null) return;
  void refreshSessionList().catch(() => undefined);
}

export function clearSessionListCache(): void {
  cachedSessions = null;
  inFlightRequest = null;
  cachedIncludeSubagents = false;
  // Same orphaning as a flag change: a request issued before the clear must not
  // repopulate the cache we just emptied.
  requestGeneration += 1;
  emitChange();
}
