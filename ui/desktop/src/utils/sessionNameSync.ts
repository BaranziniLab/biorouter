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
//      sibling-window updates themselves. Once the daemon has saved the name
//      the broadcast carries `activityAt`, and the lists sorted by activity
//      move the chat to the top at once (R-30: a rename is activity).
//   4. `subscribeSessionNameChanges` — fan-out for sibling windows
//      (history list, other open windows, the chat tab) to receive
//      updates without re-fetching.

import type { Message, Session } from '../api';
import { updateSessionName } from '../api';
import { toastError } from '../toasts';
import { errorMessage } from './conversionUtils';
import { userActionHeaders } from './userAction';

export const DEFAULT_SESSION_NAME = 'New chat';

/**
 * How many user-role messages a chat may hold and still be auto-renamed —
 * `MSG_COUNT_FOR_SESSION_NAME_GENERATION` in
 * `crates/biorouter/src/providers/base.rs`, mirrored.
 *
 * The daemon re-generates a chat's name after EVERY turn until this many user
 * messages have accumulated, and keeps trying forever while the chat is still on
 * the placeholder (`SessionManager::maybe_update_name`). It does so after the
 * reply stream has closed and announces nothing, so the renderer's post-turn
 * poll is the only thing that can notice — and this is the bound on how long
 * that poll is worth running.
 */
export const AUTO_RENAME_USER_MESSAGE_LIMIT = 3;

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
  /**
   * Set only once the daemon has SAVED the rename, to the moment it did (an ISO
   * string). A saved rename is activity in the chat (R-30): the daemon raises
   * its `updated_at`, so under "Last activity" the chat belongs at the top. The
   * lists that sort by activity (the sidebar, History, Home recents) move the
   * row when they see this, in this window and in every sibling window, instead
   * of waiting for an unrelated refresh. The optimistic announcement and a
   * rollback carry none: neither is a rename the daemon saved.
   */
  activityAt?: string;
}

/**
 * The later of a row's `updated_at` and a saved rename's `activityAt`, as the
 * string that carries it. A row a re-read already brought with the daemon's
 * own, newer time keeps it; anything unreadable leaves the row as it was.
 */
export function laterActivity(updatedAt: string, activityAt: string | undefined): string {
  if (!activityAt) return updatedAt;
  const current = Date.parse(updatedAt);
  const renamed = Date.parse(activityAt);
  if (Number.isNaN(renamed)) return updatedAt;
  return Number.isNaN(current) || renamed > current ? activityAt : updatedAt;
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
  //
  // `activityAt`: the daemon has saved the name, and a saved rename moved the
  // chat's `updated_at` (R-30). The response carries no timestamp, so this is
  // the renderer's clock; a list's next re-read carries the daemon's own.
  broadcast({
    sessionId,
    name: trimmed,
    userSetName: origin === 'user',
    origin,
    activityAt: new Date().toISOString(),
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

// ── Rename from a person (the one optimistic path) ────────────────────────

/** The server caps a chat name at 200 characters (`routes/session.rs`). */
export const SESSION_NAME_MAX_LENGTH = 200;

/** The title of the toast a failed rename shows; its message is the daemon's own text. */
export const RENAME_FAILED_TOAST_TITLE = "Couldn't rename chat";

/** What the chat was called before the rename, so a failure can put it back. */
export interface PreviousSessionName {
  name: string;
  userSetName: boolean;
}

/** `renamed`: the name was saved. `unchanged`: nothing to do, no call made. `failed`: rolled back. */
export type RenameOutcome = 'renamed' | 'unchanged' | 'failed';

/**
 * Normalise a typed or pasted name: newlines become spaces, the ends are
 * trimmed, and the server's length cap is applied so a long paste is saved
 * rather than refused.
 */
export function normalizeSessionName(raw: string): string {
  return raw
    .replace(/[\r\n]+/g, ' ')
    .trim()
    .slice(0, SESSION_NAME_MAX_LENGTH)
    .trim();
}

/**
 * Rename a chat the way a person expects: the new name shows everywhere at
 * once, and a refusal puts the old one back.
 *
 * Every rename surface (the chat title, a sidebar row, History) calls this one
 * function, so they cannot drift:
 * 1. Normalise the name. An empty or unchanged name returns `'unchanged'` and
 *    makes no request.
 * 2. Announce it optimistically with origin `'user'`, so the row, the tab and
 *    every other window update before the round trip.
 * 3. Save it through {@link renameSession}, which carries the user's proof
 *    (`userActionHeaders()`): renaming a private chat without it is refused.
 * 4. On failure, announce the previous name with origin `'sync'` and show a
 *    toast whose message is the daemon's own sentence, so a privacy refusal
 *    stays readable. There is no success toast: the row is the confirmation.
 */
export async function renameSessionOptimistically(
  sessionId: string,
  newName: string,
  previous: PreviousSessionName | null | undefined
): Promise<RenameOutcome> {
  const name = normalizeSessionName(newName);
  if (!sessionId || !name || name === previous?.name) return 'unchanged';

  announceSessionName({ sessionId, name, userSetName: true, origin: 'user' });
  try {
    await renameSession(sessionId, name, 'user');
    return 'renamed';
  } catch (err) {
    if (previous?.name) {
      announceSessionName({
        sessionId,
        name: previous.name,
        userSetName: previous.userSetName,
        origin: 'sync',
      });
    }
    toastError({ title: RENAME_FAILED_TOAST_TITLE, msg: renameFailureMessage(err) });
    return 'failed';
  }
}

/**
 * The daemon's refusal arrives as its parsed body, a plain string, under
 * `throwOnError`; an `Error` carries its own message. Anything else gets a
 * sentence rather than `[object Object]`.
 */
function renameFailureMessage(err: unknown): string {
  const fallback = 'The chat kept its old name.';
  if (typeof err === 'string') return err.trim() || fallback;
  return errorMessage(err, fallback).trim() || fallback;
}
