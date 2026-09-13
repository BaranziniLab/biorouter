import { getSession, type SessionClassification } from '../api';
import { userActionHeaders } from './userAction';

/**
 * "A chat's ROW changed in place" — for a change the list surfaces render but
 * that moves nothing in the list. Today exactly one writer needs it: a
 * declassification (issue #56 §12.4).
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
 * An open chat needs none of this: `GET /sessions/changes` compares
 * `privacy_tier` and `privacy_reason` (never `updated_at`), and its poll tells
 * the chat's controller to re-read within about two seconds.
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
 * ⚠ **Announce only what the daemon accepted.** Call it after the write
 * resolved 200; a nudge that outran its write would re-read the value it was
 * sent to replace.
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

async function reread(sessionId: string): Promise<void> {
  if (listeners.size === 0) return;
  const generation = ++readCounter;
  newestRead.set(sessionId, generation);
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
    if (newestRead.get(sessionId) !== generation) return;
    if (!row || row.id !== sessionId || !row.privacy_tier) return;
    const facts: SessionRowFacts = {
      sessionId,
      privacy_tier: row.privacy_tier,
      privacy_reason: row.privacy_reason ?? null,
    };
    for (const listener of [...listeners]) listener(facts);
  } catch {
    // Silent, as `refreshSessionBinding` is: a read that failed leaves every
    // surface exactly as stale as it was, which is not worth a toast.
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
