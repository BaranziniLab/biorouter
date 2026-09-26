import { useCallback, useSyncExternalStore } from 'react';
import type { DraftScope } from './observationFailure';

/**
 * Where a person left off in Crew, kept for when they come back within this app session (live QA
 * round 2, Q2-07 and Q2-21):
 *
 * 1. **Unsent drafts** (SECURITY-SENSITIVE, human review). Switching channel or team, switching
 *    workspace or leaving Crew used to throw the composer's text away. The body is now kept here
 *    with the scope it was written under, and handed back on the first verified view of the same
 *    channel only when `draftScopeChanged` says nothing it was written under moved.
 *    - Memory only: a module-level map, never `localStorage`, so nothing survives the app.
 *    - The body only. Attachments, references and context channels carry capabilities (an
 *      uploaded blob, a server path, another channel's content) and are never kept.
 *    - Bounded: at most `DRAFT_STASH_MAX_ENTRIES` entries, the oldest dropped first, and a body
 *      over `DRAFT_STASH_MAX_BODY_BYTES` is not kept at all rather than cut short.
 *    - A draft with no verified scope for its own channel is not kept: it could not be checked
 *      before it came back.
 *    - The rail marks a channel that holds one (Q3-09) through `useChannelHasDraft`: whether a
 *      body is kept, never the body itself.
 * 2. **The last channel** the person chose, per connection: the channel ID only, in memory and in
 *    `localStorage` (`crew:lastChannel:<connectionId>`), so Crew reopens where they were instead
 *    of on the team's first channel. It only ever picks among the channels a verified view offers.
 */

/** The most drafts kept at once; the oldest goes first. */
export const DRAFT_STASH_MAX_ENTRIES = 50;
/** The largest body kept, in UTF-8 bytes. A longer one is not kept. */
export const DRAFT_STASH_MAX_BODY_BYTES = 64 * 1024;

export interface StashedDraft {
  body: string;
  /** What the draft was written under: the last verified view of its channel. */
  scope: DraftScope;
}

const drafts = new Map<string, StashedDraft>();
const draftListeners = new Set<() => void>();

function draftKey(connectionId: string, channelId: string): string {
  return `${connectionId}\n${channelId}`;
}

function notifyDrafts(): void {
  for (const listener of [...draftListeners]) listener();
}

/** Call `listener` whenever a kept draft is added or goes. Returns the unsubscribe. */
export function subscribeDrafts(listener: () => void): () => void {
  draftListeners.add(listener);
  return () => {
    draftListeners.delete(listener);
  };
}

/** Whether `channelId` on `connectionId` holds a kept unsent draft (its text only). */
export function useChannelHasDraft(connectionId: string, channelId: string): boolean {
  const read = useCallback(
    () => Boolean(connectionId && channelId && drafts.has(draftKey(connectionId, channelId))),
    [connectionId, channelId]
  );
  return useSyncExternalStore(subscribeDrafts, read, read);
}

function bodyBytes(body: string): number {
  return new TextEncoder().encode(body).length;
}

/**
 * Keep `body` as the unsent draft of `channelId` on `connectionId`, written under `scope`.
 *
 * Nothing is kept — and nothing already kept is touched — for an empty body, a missing
 * connection or channel, or a scope that is not the last verified view of this very channel on
 * this very connection. A body over the size bound replaces nothing and is dropped.
 */
export function stashDraft(
  connectionId: string,
  channelId: string,
  body: string,
  scope: DraftScope | null
): void {
  if (!connectionId || !channelId || !body.trim()) return;
  if (!scope || scope.connectionId !== connectionId || scope.channel?.id !== channelId) return;
  const key = draftKey(connectionId, channelId);
  const replaced = drafts.delete(key);
  if (bodyBytes(body) > DRAFT_STASH_MAX_BODY_BYTES) {
    if (replaced) notifyDrafts();
    return;
  }
  drafts.set(key, { body, scope });
  while (drafts.size > DRAFT_STASH_MAX_ENTRIES) {
    const oldest = drafts.keys().next().value;
    if (oldest === undefined) break;
    drafts.delete(oldest);
  }
  notifyDrafts();
}

/** The kept draft of a channel, if any, without taking it. */
export function stashedDraft(connectionId: string, channelId: string): StashedDraft | undefined {
  return drafts.get(draftKey(connectionId, channelId));
}

/** Take (return and forget) the kept draft of a channel. */
export function takeStashedDraft(
  connectionId: string,
  channelId: string
): StashedDraft | undefined {
  const key = draftKey(connectionId, channelId);
  const entry = drafts.get(key);
  if (drafts.delete(key)) notifyDrafts();
  return entry;
}

/** Forget the kept draft of one channel (it was sent, or the channel was lost). */
export function forgetStashedDraft(connectionId: string, channelId: string): void {
  if (drafts.delete(draftKey(connectionId, channelId))) notifyDrafts();
}

/**
 * Forget every kept draft of a connection: its privacy or access changed, or it was removed.
 * With `keep`, a draft stays only if its channel passes (a verified view still offers it).
 */
export function forgetConnectionDrafts(
  connectionId: string,
  keep?: (channelId: string) => boolean
): void {
  const prefix = `${connectionId}\n`;
  let changed = false;
  for (const key of [...drafts.keys()]) {
    if (!key.startsWith(prefix)) continue;
    if (keep?.(key.slice(prefix.length))) continue;
    drafts.delete(key);
    changed = true;
  }
  if (changed) notifyDrafts();
}

/** How many drafts are kept. For tests and diagnostics; never shown. */
export function stashedDraftCount(): number {
  return drafts.size;
}

// ---------------------------------------------------------------------------------------------
// The last channel per connection
// ---------------------------------------------------------------------------------------------

export const LAST_CHANNEL_STORAGE_PREFIX = 'crew:lastChannel:';
/** A channel ID is a UUID; anything much longer in storage is not one of ours. */
const MAX_STORED_CHANNEL_ID_LENGTH = 128;

const lastChannels = new Map<string, string>();

/** Remember `channelId` as the channel the person last chose on `connectionId`. */
export function rememberLastChannel(connectionId: string, channelId: string): void {
  if (!connectionId || !channelId) return;
  lastChannels.set(connectionId, channelId);
  try {
    window.localStorage.setItem(`${LAST_CHANNEL_STORAGE_PREFIX}${connectionId}`, channelId);
  } catch {
    // Storage refused: the in-memory copy still serves this app session.
  }
}

/** The channel the person last chose on `connectionId` in this session or an earlier one. */
export function rememberedLastChannel(connectionId: string): string | null {
  if (!connectionId) return null;
  const remembered = lastChannels.get(connectionId);
  if (remembered) return remembered;
  try {
    const stored = window.localStorage.getItem(`${LAST_CHANNEL_STORAGE_PREFIX}${connectionId}`);
    return stored && stored.length <= MAX_STORED_CHANNEL_ID_LENGTH ? stored : null;
  } catch {
    return null;
  }
}

/** Forget the last channel of a removed connection. */
export function forgetLastChannel(connectionId: string): void {
  lastChannels.delete(connectionId);
  try {
    window.localStorage.removeItem(`${LAST_CHANNEL_STORAGE_PREFIX}${connectionId}`);
  } catch {
    // Nothing more to forget.
  }
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

/**
 * Start every test from nothing (vitest only). A test file shares one instance of each module
 * across its tests, so memory kept here for the app session would reach the next test: a channel
 * the last test chose would open instead of the first, a draft it left would come back. The chat
 * composer's parked queues get the same treatment from `src/test/setup.ts`; this registers from
 * the module itself, with vitest's global `beforeEach` (`globals: true`), so every test file that
 * mounts the controller is covered. Outside vitest it does nothing, and the build drops it.
 */
export function resetBetweenTests(reset: () => void): void {
  if (import.meta.env.MODE !== 'test') return;
  const beforeEachTest = (globalThis as { beforeEach?: (hook: () => void) => void }).beforeEach;
  try {
    beforeEachTest?.(reset);
  } catch {
    // Imported from inside a running test: that test starts from whatever it set up itself.
  }
}

/**
 * Forget everything this module keeps: every draft, and every remembered channel (in memory and
 * in storage).
 */
export function resetDraftStashForTests(): void {
  const had = drafts.size > 0;
  drafts.clear();
  if (had) notifyDrafts();
  for (const connectionId of [...lastChannels.keys()]) forgetLastChannel(connectionId);
  try {
    const stale: string[] = [];
    for (let index = 0; index < window.localStorage.length; index += 1) {
      const key = window.localStorage.key(index);
      if (key?.startsWith(LAST_CHANNEL_STORAGE_PREFIX)) stale.push(key);
    }
    for (const key of stale) window.localStorage.removeItem(key);
  } catch {
    // No storage to clear.
  }
}

resetBetweenTests(resetDraftStashForTests);
