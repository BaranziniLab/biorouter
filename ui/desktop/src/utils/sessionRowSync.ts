import { getSession, type SessionClassification } from '../api';
import { userActionHeaders } from './userAction';

/**
 * "A chat's ROW changed in place" — for a change the list surfaces render but
 * that moves nothing in the list: a chat's classification moving in EITHER
 * direction. A declassification (issue #56 §12.4) announces itself from the
 * dialog that made it; a raise is announced by whichever window's chat store
 * first sees it (`ChatStreamRegistry.noteControllerTier`).
 *
 * # Why this exists — item 11 of the 1.90.4 hold (2026-09-13)
 *
 * `privacy::declassify` used to stamp `updated_at = datetime('now')`, which put
 * a chat created months ago into History's "Today" (measured across 796
 * declassifications). The stamp is gone, and the obvious worry was that
 * something learned about a declassification *because* `updated_at` moved.
 * Measured before removing it, with History open in two windows: window A
 * declassified `20260803_1550`, and window B went on badging it private for the
 * full 30 seconds watched. Nothing crossed. B learned only when some unrelated
 * refresh re-read a list — which the stamp had re-sorted, so the chat came back
 * at the TOP. Without the stamp that incidental path is gone for the sidebar
 * outright: `useSidebarSessions` re-reads only the head of its keyset, and a
 * months-old row is never in the head.
 *
 * So the change announces itself, and every list surface re-reads the row in
 * place:
 *
 * - `sessionListCache` (History's rows, Home recents, the tab strip's cached
 *   tiers) patches the entry it holds;
 * - `useSidebarSessions` patches the row it holds, wherever it sits;
 * - `SessionHistoryView` moves its page badge.
 *
 * An open chat's store is followed by `GET /sessions/changes`, which compares
 * `privacy_tier` and `privacy_reason` (never `updated_at`), and its poll tells
 * the chat's controller to re-read within about two seconds. The registry also
 * hands a controller every row read made here, so a store that holds a stale
 * tier re-reads at once rather than on the next poll.
 *
 * # Both directions, or the push is a new way to be wrong
 *
 * ⚠ **Pushing only the lowering was defect D4 of the 2026-09-13 repair round.**
 * Before this channel existed a second window never learned of a
 * declassification, so it kept a private badge — stale, but in the safe
 * direction. Once the lowering was pushed and nothing pushed the raise, the
 * sequence "declassify in A, then send one turn on a private model in A" left
 * window B's History row AND its sidebar row badging the chat PUBLIC for as
 * long as it was watched (90 s, and the sidebar over two minutes), while the
 * database read `private` / `turn:versa_azure`. A channel that can lower a
 * badge must also be able to raise it, so a store that observes its chat's tier
 * change announces here too — see `ChatStreamRegistry.noteControllerTier`.
 *
 * {@link lastKnownSessionTier} is what keeps that from echoing: a window that
 * has already delivered a read of the new tier (because someone announced it)
 * does not announce the same change again when its own store catches up.
 *
 * # The announcement is a nudge, never a payload
 *
 * The same rule `sessionBindingSync`'s app-model nudge and `sessionMetaSubscription`
 * state: a receiver is handed an id and goes to look. A declassified chat can be
 * raised straight back by a turn a moment later, and a window that applied
 * "public" from a message would show a public badge over a private chat — the
 * one direction a privacy badge must never be wrong in. Re-reading the daemon
 * ends on whichever WRITE landed last.
 *
 * ⚠ **One read per announcement per window, shared by every subscriber.** The
 * row is read here and handed to the subscribers, rather than each subscriber
 * fetching for itself, so a declassification costs one `GET` per window.
 *
 * ⚠ **Announce only what has landed.** Call it after the write resolved (or a
 * read showed that it had), or after a store READ the new tier; a nudge that
 * outran its write would re-read the value it was sent to replace.
 */

export interface SessionRowFacts {
  sessionId: string;
  privacy_tier: SessionClassification;
  privacy_reason: string | null;
}

type Listener = (facts: SessionRowFacts) => void;

const listeners = new Set<Listener>();

/**
 * The newest read issued for each chat. Two announcements for one chat can
 * overlap, and the answer to the first may arrive second; only the newest read
 * is delivered. Generations come from one counter that never resets, so a read
 * issued after an entry was cleared can never share a number with an older one
 * still in flight.
 */
const newestRead = new Map<string, number>();
let readCounter = 0;

/**
 * The tier of the last read this window DELIVERED for each chat. See
 * {@link lastKnownSessionTier}. Holds one short string per chat the channel has
 * carried, which is bounded by the chats whose classification moved.
 */
const delivered = new Map<string, SessionClassification>();

let channel: BroadcastChannel | null = null;

function getChannel(): BroadcastChannel | null {
  if (channel) return channel;
  // Lazy, and absent-tolerant, so a test environment without BroadcastChannel
  // still loads the module — as `sessionNameSync` and `sessionBindingSync` do.
  if (typeof BroadcastChannel === 'undefined') return null;
  channel = new BroadcastChannel('biorouter:session-row');
  channel.onmessage = (event: MessageEvent) => {
    // Shape-checked, not trusted: this arrives from another window.
    const sessionId = (event.data as { sessionId?: unknown } | null | undefined)?.sessionId;
    if (typeof sessionId !== 'string' || sessionId.length === 0) return;
    void reread(sessionId);
  };
  return channel;
}

/**
 * Read one chat's classification from the daemon — the row, not the
 * transcript, with the proof-of-user a private chat's row needs.
 *
 * `null` when the read failed or answered for another chat. Never throws: a
 * caller that needs to know what happened to a write asks this, and "could not
 * ask" is an answer it has to handle rather than an exception it can forget.
 */
export async function readSessionRowFacts(sessionId: string): Promise<SessionRowFacts | null> {
  try {
    const response = await getSession({
      path: { session_id: sessionId },
      // The row, not the transcript: this wants two strings.
      query: { metadata_only: true },
      // A private chat's row is refused without the proof-of-user, and the
      // chat this exists for may still be private by the time it is read.
      headers: await userActionHeaders(),
      throwOnError: true,
    });
    const row = response.data;
    if (!row || row.id !== sessionId || !row.privacy_tier) return null;
    return {
      sessionId,
      privacy_tier: row.privacy_tier,
      privacy_reason: row.privacy_reason ?? null,
    };
  } catch {
    return null;
  }
}

async function reread(sessionId: string): Promise<void> {
  if (listeners.size === 0) return;
  const generation = ++readCounter;
  newestRead.set(sessionId, generation);
  try {
    const facts = await readSessionRowFacts(sessionId);
    // Silent on failure, as `refreshSessionBinding` is: a read that failed
    // leaves every surface exactly as stale as it was, which is not worth a
    // toast.
    if (!facts || newestRead.get(sessionId) !== generation) return;
    delivered.set(sessionId, facts.privacy_tier);
    for (const listener of [...listeners]) listener(facts);
  } finally {
    if (newestRead.get(sessionId) === generation) newestRead.delete(sessionId);
  }
}

/**
 * Announce that `sessionId`'s row changed in place. Re-reads it for this window
 * and tells every other window to do the same.
 */
export function announceSessionRowChanged(sessionId: string): void {
  void reread(sessionId);
  getChannel()?.postMessage({ sessionId });
}

/**
 * Re-read `sessionId`'s row for THIS window only, and deliver it to this
 * window's subscribers. No other window is told.
 *
 * For a disagreement this window found by itself: a list answer that differs
 * from a row read which landed while that list was in flight. Nobody else holds
 * that pair of readings, so nobody else needs the third one that settles it.
 */
export function rereadSessionRowHere(sessionId: string): void {
  void reread(sessionId);
}

/**
 * Reconcile a LIST answer with the row reads this window delivered while that
 * list request was in flight, and return the rows to publish.
 *
 * # Why a list surface cannot just take either one
 *
 * Neither reading is known to be the later one. A list request issued before a
 * turn raised a chat can land after the raise was read and patched in, and a
 * surface that adopted the list would put the chat back to PUBLIC — a raise
 * undone by an older photograph. Replaying the row read over the list is wrong
 * the other way: that read may itself have been issued before the list, so it
 * could write an older `public` over a newer `private`.
 *
 * So a disagreement is settled the only way that does not guess:
 *
 * 1. until it is settled the row shows the HIGHER tier of the two (a chat is
 *    never drawn public while either reading says private), and
 * 2. the row is read a third time, now — after both — and that read patches
 *    the surface through the ordinary channel ({@link rereadSessionRowHere}).
 *
 * A row the two readings agree on, and a row no read touched, is published as
 * the list answered.
 *
 * `withReason` compares `privacy_reason` as well, for a surface whose rows carry
 * it (`Session`); `SessionSummary` carries only the tier.
 *
 * Consumes `readDuringFetch` (it is cleared).
 */
export function settleRowsReadDuringFetch<
  T extends {
    id: string;
    privacy_tier?: SessionClassification | null;
    privacy_reason?: string | null;
  },
>(rows: T[], readDuringFetch: Map<string, SessionRowFacts>, withReason: boolean): T[] {
  if (readDuringFetch.size === 0) return rows;
  const unsettled: string[] = [];
  const settled = rows.map((row) => {
    const read = readDuringFetch.get(row.id);
    if (!read) return row;
    const agrees =
      row.privacy_tier === read.privacy_tier &&
      (!withReason || (row.privacy_reason ?? null) === read.privacy_reason);
    if (agrees) return row;
    unsettled.push(row.id);
    if (read.privacy_tier !== 'private' || row.privacy_tier === 'private') return row;
    return withReason
      ? { ...row, privacy_tier: read.privacy_tier, privacy_reason: read.privacy_reason }
      : { ...row, privacy_tier: read.privacy_tier };
  });
  readDuringFetch.clear();
  for (const sessionId of unsettled) rereadSessionRowHere(sessionId);
  return settled;
}

/**
 * The tier of the last read of `sessionId` this window delivered to its
 * subscribers, or `undefined` when the channel has carried nothing for it.
 *
 * What it is for: a chat store that sees its tier change asks this before
 * announcing, and says nothing when the window has already delivered that tier
 * — the change was announced by whoever made it, every window has read it, and
 * a second announcement would only make every window read it again.
 */
export function lastKnownSessionTier(sessionId: string): SessionClassification | undefined {
  return delivered.get(sessionId);
}

/**
 * Subscribe to freshly-read rows. Returns the unsubscribe. Subscribe from a
 * MOUNT, or — for a module that owns a cache, as `sessionListCache` does its
 * name channel — at module scope; never from a getter.
 */
export function subscribeSessionRowChanges(listener: Listener): () => void {
  getChannel();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
