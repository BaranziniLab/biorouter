import { listSessions, type Session } from '../api';
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

// A session rename rides the name channel, not the list channel — but the
// See-all view and Home recents read THIS cache, so patch the cached name when
// one arrives. Without this a rename made in the tab pill or the sidebar never
// reached those two surfaces until they remounted, and the same session showed
// two different names in two panels at once.
subscribeSessionNameChanges(({ sessionId, name, userSetName }) => {
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
  inFlightRequest = listSessions<true>({
    throwOnError: true,
    // `cachedIncludeSubagents`, not the parameter: a keyless call must send the
    // identity the cache is holding, not `undefined`.
    query: { include_subagents: cachedIncludeSubagents },
  })
    .then((response) => {
      // Superseded while in flight: hand the answer back to whoever awaited
      // this exact call, but publish nothing — the cache and its subscribers
      // belong to the request that replaced it.
      if (generation !== requestGeneration) return response.data.sessions;
      cachedSessions = response.data.sessions;
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
