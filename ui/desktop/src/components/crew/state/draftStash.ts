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
 *    - Bounded: at most `DRAFT_STASH_MAX_ENTRIES` entries and `DRAFT_STASH_MAX_TOTAL_BYTES` of
 *      text, the oldest dropped first, and a body over `DRAFT_STASH_MAX_BODY_BYTES` is not kept at
 *      all rather than cut short. That bound is well above a message's 64 KB, so the very drafts
 *      the composer's "Messages can be up to 64 KB" note is about are kept with it (MSG2-N4); the
 *      composer says so while a draft is too long to keep (`draftTooLongToKeep`).
 *    - A draft with no verified scope for its own channel is not kept: it could not be checked
 *      before it came back.
 *    - The rail marks a channel that holds one (Q3-09) through `useChannelHasDraft`: whether a
 *      body is kept, never the body itself.
 *    - With the message attempt made for that very body, when it was sent and no answer cleared it
 *      (RENDERER-4): the idempotency key it went with, so sending it again after coming back is the
 *      same message to the broker, never a second one. The attempt is kept in the entry itself, so
 *      it is kept exactly as long as the body is and every door that forgets the body forgets it.
 *      It holds a SHA-256 digest of what was posted and the key, never the attachment or
 *      reference IDs themselves.
 *    - With the composer's note about that body, when it had one (QA M5): the words of a send
 *      failure, so the failure comes back with its draft rather than following the person into
 *      another channel. Words only, never a capability.
 * 2. **The last channel** the person chose, per connection: the channel ID only, in memory and in
 *    `localStorage` (`crew:lastChannel:<connectionId>`), so Crew reopens where they were instead
 *    of on the team's first channel. It only ever picks among the channels a verified view offers.
 */

/** The most drafts kept at once; the oldest goes first. */
export const DRAFT_STASH_MAX_ENTRIES = 50;
/**
 * The largest body kept, in UTF-8 bytes. A longer one is not kept. It was 64 KiB, the message
 * limit itself, so a draft over the limit, the one the composer tells the person to attach as a
 * file, went the moment they switched channel, and the note with it (MSG2-N4).
 */
export const DRAFT_STASH_MAX_BODY_BYTES = 1024 * 1024;
/** The most text kept across every draft, in UTF-8 bytes; the oldest goes first. */
export const DRAFT_STASH_MAX_TOTAL_BYTES = 8 * 1024 * 1024;

/** One `message.post` attempt: a digest of what it carried, and the idempotency key it used. */
export interface MessageAttempt {
  /** SHA-256, as hex, of the post's fingerprint; never the attachment or reference IDs. */
  digest: string;
  key: string;
  /**
   * The broker took the post after the composer that sent it had moved on, and the words were
   * still held: sending them again is this message. The first write to them lets it go.
   */
  delivered?: boolean;
}

/**
 * The attempt that belongs to one text, wherever that text is: the composer, or this stash. The
 * same object goes with the text from one to the other, so a post that fills it in once its
 * fingerprint is known reaches the text even when the text was put aside meanwhile (a channel
 * switch, or leaving Crew, while the post was being prepared).
 */
export interface DraftAttempt {
  current: MessageAttempt | null;
}

/** The composer's note about a kept draft: what its last send said, in words. */
export interface StashedNote {
  message: string;
  code?: string;
  transport?: boolean;
}

export interface StashedDraft {
  body: string;
  /** What the draft was written under: the last verified view of its channel. */
  scope: DraftScope;
  /** The attempt made for this very body, when it was sent and no answer cleared it. */
  attempt?: DraftAttempt;
  /** What the composer said about this body when it was put aside. */
  note?: StashedNote;
}

const drafts = new Map<string, StashedDraft>();
/** Each kept body's size in UTF-8 bytes, by the same key, for the total bound. */
const draftBytes = new Map<string, number>();
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
 * Whether `body` is too long to be kept when the person switches channel or leaves Crew. The
 * composer says so while it is (MSG2-N4), before the words could go. A UTF-16 unit is at least one
 * UTF-8 byte and at most three, so only a body between the two bounds is encoded.
 */
export function draftTooLongToKeep(body: string): boolean {
  if (body.length * 3 <= DRAFT_STASH_MAX_BODY_BYTES) return false;
  if (body.length > DRAFT_STASH_MAX_BODY_BYTES) return true;
  return bodyBytes(body) > DRAFT_STASH_MAX_BODY_BYTES;
}

/** Forget one kept draft and its size. Returns whether one was kept. */
function dropDraft(key: string): boolean {
  draftBytes.delete(key);
  return drafts.delete(key);
}

/**
 * Keep `body` as the unsent draft of `channelId` on `connectionId`, written under `scope`, with
 * `attempt` when one was made for this very body, and `note` when the composer had one about it.
 *
 * Nothing is kept — and nothing already kept is touched — for an empty body, a missing
 * connection or channel, or a scope that is not the last verified view of this very channel on
 * this very connection. A body over the size bound replaces nothing and is dropped, and its
 * attempt with it.
 */
export function stashDraft(
  connectionId: string,
  channelId: string,
  body: string,
  scope: DraftScope | null,
  attempt?: DraftAttempt | null,
  note?: StashedNote | null
): void {
  if (!connectionId || !channelId || !body.trim()) return;
  if (!scope || scope.connectionId !== connectionId || scope.channel?.id !== channelId) return;
  const key = draftKey(connectionId, channelId);
  const replaced = dropDraft(key);
  const size = draftTooLongToKeep(body) ? Number.POSITIVE_INFINITY : bodyBytes(body);
  if (size > DRAFT_STASH_MAX_BODY_BYTES) {
    if (replaced) notifyDrafts();
    return;
  }
  drafts.set(key, {
    body,
    scope,
    ...(attempt ? { attempt } : {}),
    ...(note ? { note } : {}),
  });
  draftBytes.set(key, size);
  let total = 0;
  for (const bytes of draftBytes.values()) total += bytes;
  while (drafts.size > DRAFT_STASH_MAX_ENTRIES || total > DRAFT_STASH_MAX_TOTAL_BYTES) {
    const oldest = drafts.keys().next().value;
    if (oldest === undefined || oldest === key) break;
    total -= draftBytes.get(oldest) ?? 0;
    dropDraft(oldest);
  }
  notifyDrafts();
}

/**
 * Write the composer's note about the kept draft of a channel, when one is kept: a refusal of a
 * post that answered while no Crew screen was open, told when the draft comes back rather than
 * never. Nothing is kept for a channel that keeps no draft.
 */
export function noteKeptDraft(connectionId: string, channelId: string, note: StashedNote): void {
  const key = draftKey(connectionId, channelId);
  const entry = drafts.get(key);
  if (entry) drafts.set(key, { ...entry, note });
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
  if (dropDraft(key)) notifyDrafts();
  return entry;
}

/** Forget the kept draft of one channel, and its attempt (it was sent, or the channel was lost). */
export function forgetStashedDraft(connectionId: string, channelId: string): void {
  if (dropDraft(draftKey(connectionId, channelId))) notifyDrafts();
}

/**
 * Forget every kept draft of a connection, and their attempts: its privacy or access changed, or
 * it was removed. With `keep`, a draft stays only if its channel passes (a verified view still
 * offers it). Returns what was forgotten, by channel, so a caller whose reason is a lost channel
 * can offer the person their own words once more (QA M10); none is ever put back in a composer.
 */
export function forgetConnectionDrafts(
  connectionId: string,
  keep?: (channelId: string) => boolean
): { channelId: string; body: string }[] {
  const prefix = `${connectionId}\n`;
  const forgotten: { channelId: string; body: string }[] = [];
  for (const [key, entry] of [...drafts.entries()]) {
    if (!key.startsWith(prefix)) continue;
    const channelId = key.slice(prefix.length);
    if (keep?.(channelId)) continue;
    dropDraft(key);
    forgotten.push({ channelId, body: entry.body });
  }
  if (forgotten.length > 0) notifyDrafts();
  return forgotten;
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
  draftBytes.clear();
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
