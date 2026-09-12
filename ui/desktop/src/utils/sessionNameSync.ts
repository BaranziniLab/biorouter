// Single source of truth for session-name state across the renderer.
//
// Why this exists: the rename path used to live in three places — BaseChat's
// chat-tab pill, the dashboard window title-bar (since removed), and
// useChatStream's LLM-auto-rename polling. Each one called the API directly,
// none updated the in-memory session cache, and each had its own regex-based
// "is this a default placeholder?" check duplicated from useChatStream. The
// result was the snap-back bug: a name committed in the UI would stick
// momentarily, then revert when the stale cache or stale localStorage state
// won a race.
//
// This module centralizes:
//   1. The cache (cacheGet / cacheSet / cacheUpdateName) so renames update
//      every caller's view of `session` consistently.
//   2. The definition of a "default" session name. Backend assigns
//      `'New chat'` to every freshly-created session (see
//      crates/biorouter-server/src/routes/agent.rs); legacy sessions may
//      still carry numbered variants like `'New session 154'` or
//      `'Session 5'`. `isDefaultSessionName` accepts the current and legacy
//      forms so older
//      data continues to read correctly.
//   3. `renameSession` — optimistic API call + cache update + cross-window
//      broadcast. Callers do not need to coordinate cache invalidation or
//      sibling-window updates themselves.
//   4. `subscribeSessionNameChanges` — fan-out for sibling windows
//      (history list, other open windows, the chat tab) to receive
//      updates without re-fetching.

import type { Message, Session } from '../api';
import { updateSessionName } from '../api';
import { userActionHeaders } from './userAction';

export const DEFAULT_SESSION_NAME = 'New chat';

const DEFAULT_NAME_PATTERNS: RegExp[] = [
  /^New chat$/i,
  /^New Session$/i,
  /^CLI Session$/i,
  /^New session \d+$/i,
  /^Session \d+$/i,
];

export function isDefaultSessionName(name: string | null | undefined): boolean {
  if (!name) return true;
  return DEFAULT_NAME_PATTERNS.some((re) => re.test(name));
}

// ── Cache ─────────────────────────────────────────────────────────────────
// LRU cap mirrors the previous value in useChatStream.ts. Without it heavy
// users accumulated every conversation they'd opened for the renderer's
// lifetime.

const RESULTS_CACHE_MAX = 10;
type CacheEntry = { messages: Message[]; session: Session };
const resultsCache = new Map<string, CacheEntry>();

export function cacheGet(key: string): CacheEntry | undefined {
  const v = resultsCache.get(key);
  if (v) {
    // Reorder for LRU.
    resultsCache.delete(key);
    resultsCache.set(key, v);
  }
  return v;
}

export function cacheSet(key: string, value: CacheEntry): void {
  if (resultsCache.has(key)) resultsCache.delete(key);
  resultsCache.set(key, value);
  while (resultsCache.size > RESULTS_CACHE_MAX) {
    const oldest = resultsCache.keys().next().value;
    if (oldest === undefined) break;
    resultsCache.delete(oldest);
  }
}

/** Patch the cached session for `sessionId` with a new name / user_set_name
 * without forcing a full session refetch. No-op if the session isn't cached. */
export function cacheUpdateName(sessionId: string, name: string, userSetName: boolean): void {
  const entry = resultsCache.get(sessionId);
  if (!entry) return;
  resultsCache.delete(sessionId);
  resultsCache.set(sessionId, {
    ...entry,
    session: { ...entry.session, name, user_set_name: userSetName },
  });
}

// ── Cross-window broadcast ────────────────────────────────────────────────
// BroadcastChannel works inside a single renderer (across React subtrees,
// open routes, sibling subtrees) and also across BrowserWindows in
// the same Electron app (Chromium broadcasts on a per-origin basis).

export interface SessionNameChange {
  sessionId: string;
  name: string;
  userSetName: boolean;
  /** Where the rename originated. Used so the originating window can ignore
   *  its own broadcast and avoid a no-op re-render. */
  origin: 'user' | 'llm' | 'sync';
}

type Listener = (change: SessionNameChange) => void;
const listeners = new Set<Listener>();

let channel: BroadcastChannel | null = null;
function getChannel(): BroadcastChannel | null {
  if (channel) return channel;
  if (typeof BroadcastChannel === 'undefined') return null;
  channel = new BroadcastChannel('biorouter:session-name');
  channel.onmessage = (event: MessageEvent) => {
    const change = event.data as SessionNameChange | undefined;
    if (!change || !change.sessionId || typeof change.name !== 'string') return;
    cacheUpdateName(change.sessionId, change.name, change.userSetName);
    for (const l of listeners) l(change);
  };
  return channel;
}

export function subscribeSessionNameChanges(listener: Listener): () => void {
  getChannel(); // lazy-init so test environments without BroadcastChannel
  // (jsdom < 22) still load the module.
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function broadcast(change: SessionNameChange): void {
  cacheUpdateName(change.sessionId, change.name, change.userSetName);
  for (const l of listeners) l(change);
  getChannel()?.postMessage(change);
}

// ── Rename ────────────────────────────────────────────────────────────────

/** Persist a rename to the backend and propagate it to every other view in
 *  the renderer. Returns the saved name on success. Throws on API failure;
 *  callers that did an optimistic UI update should roll back in `catch`. */
export async function renameSession(
  sessionId: string,
  newName: string,
  origin: SessionNameChange['origin'] = 'user'
): Promise<string> {
  const trimmed = newName.trim();
  if (!trimmed) throw new Error('Chat name cannot be empty');

  // With the user's proof: renaming a private chat is refused, exactly as
  // reading it is, to a caller without it (issue #56, QA 2026-09-10).
  await updateSessionName({
    path: { session_id: sessionId },
    body: { name: trimmed },
    headers: await userActionHeaders(),
    throwOnError: true,
  });

  // A user rename always sets user_set_name=true on the backend; an LLM
  // rename leaves it false. `origin` tells the broadcast which is which so
  // listeners can decide whether to allow future LLM auto-renames.
  broadcast({
    sessionId,
    name: trimmed,
    userSetName: origin === 'user',
    origin,
  });

  return trimmed;
}

/** Lower-level: announce a name change that has already happened on the
 *  backend (e.g. observed via polling or via resumeAgent). Useful when the
 *  LLM auto-rename completes — we want listeners to update without doing
 *  another PUT. */
export function announceSessionName(change: SessionNameChange): void {
  broadcast(change);
}
