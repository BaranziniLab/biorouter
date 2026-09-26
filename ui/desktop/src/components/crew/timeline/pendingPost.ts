import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react';
import type { CrewMessage } from '../crewApi';
import { crewActionCopy } from '../state/copy';
import type { CrewController } from '../state/types';

/**
 * The post between Send and its arrival (T-37), kept where more than the timeline can read it.
 *
 * The send is not optimistic — the draft stays until the broker answers — but once it has answered
 * the draft is cleared, and the message arrives only when the observer next delivers it, seconds
 * later. From the answer until the message is in the list:
 * - the timeline draws a dimmed "Sending…" row for it, with its files and, when it is the day's
 *   first message, the day's band, so nothing moves when it lands (Q4-19);
 * - the Files tab keeps its files under "In your message", reading "Sending…" (Q4-17). They used
 *   to be in no section for a second or two: out of the draft, and in no loaded message yet.
 *
 * The timeline works it out ({@link usePendingPost}) and publishes it here per connection and
 * channel; the Files tab reads it ({@link usePendingPostOf}). Display only: it decides nothing
 * about the post, which the broker has already accepted.
 */

/** A file in the post, by the name the draft gave it. */
export interface PendingPostFile {
  id: string;
  name: string;
}

/** A post the broker has accepted, from this channel's composer. */
export interface PendingPost {
  body: string;
  /** The files it carries, in the draft's order. */
  attachments: readonly PendingPostFile[];
  /** Messages already on screen when it was sent: the delivered one is not among them. */
  before: ReadonlySet<string>;
}

/**
 * How long a "Sending…" row waits for the observer to deliver its message
 * before it goes quietly. The broker accepted the post, so the message is
 * coming; this only keeps a stalled observation from leaving the row for good.
 */
export const PENDING_POST_TIMEOUT_MS = 30_000;

/** The draft as a send began: what to recognize the delivered message by. */
type PostAttempt = PendingPost;

/**
 * Whether the send that just settled was accepted. The controller's `send()`
 * says nothing (and `state/*` is not this area's to change), so it is read the
 * way the composer sees it: an accepted post clears exactly what was sent from
 * the draft, and a refused one leaves the draft and records a composer error. A
 * post whose only trouble was the kept upload record still went out. A draft
 * cleared because the verified view was dropped proves nothing either way.
 */
function postAccepted(attempt: PostAttempt, crew: CrewController): boolean {
  const { error, draft, snapshot } = crew;
  if (error?.source === 'composer' && error.message !== crewActionCopy.sendTransferRecordKept) {
    return false;
  }
  // A reset that dropped the verified view (and cleared the draft with it) is not an answer.
  if (!snapshot) return false;
  const bodyCleared = !attempt.body.trim() || !draft.body.trim();
  const filesCleared = attempt.attachments.every(
    (sent) => !draft.attachments.some((file) => file.id === sent.id)
  );
  return bodyCleared && filesCleared;
}

function isDelivery(post: PendingPost, message: CrewMessage, viewerId: string | null): boolean {
  return (
    viewerId !== null &&
    !post.before.has(message.id) &&
    message.actor_id === viewerId &&
    !message.run_id &&
    message.body.trim() === post.body.trim()
  );
}

// ── The store the Files tab reads ──────────────────────────────────────────

const published = new Map<string, PendingPost>();
const listeners = new Set<() => void>();

const storeKey = (connectionId: string, channelId: string) => `${connectionId}\n${channelId}`;

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function publish(key: string, post: PendingPost | null) {
  if (post) published.set(key, post);
  else published.delete(key);
  for (const listener of listeners) listener();
}

/** The post on its way in this channel, as the timeline last published it; null when none. */
export function usePendingPostOf(connectionId: string, channelId: string): PendingPost | null {
  const key = storeKey(connectionId, channelId);
  return useSyncExternalStore(subscribe, () => published.get(key) ?? null);
}

/**
 * The post on its way from this timeline's composer, matched on arrival by who posted it and its
 * words among messages that were not already on screen (`send()` returns no message ID to match
 * by). Published for the Files tab under the controller's connection and channel while it lasts.
 *
 * The answer is taken in a layout effect, so the draft emptying and the "Sending…" row appearing
 * land in one painted frame, and the Files tab's rows move from the draft to the post in the same
 * one.
 */
export function usePendingPost(
  crew: CrewController,
  messages: readonly CrewMessage[],
  viewerId: string | null
): PendingPost | null {
  const posting = crew.isPending('send');
  const [pending, setPending] = useState<PendingPost | null>(null);
  const attempt = useRef<PostAttempt | null>(null);
  const wasPosting = useRef(false);
  const latest = useRef({ crew, messages });
  latest.current = { crew, messages };
  useLayoutEffect(() => {
    const was = wasPosting.current;
    wasPosting.current = posting;
    const { crew: now, messages: list } = latest.current;
    if (posting && !was) {
      attempt.current = {
        body: now.draft.body,
        attachments: now.draft.attachments.map((file) => ({ id: file.id, name: file.name })),
        before: new Set(list.map((message) => message.id)),
      };
    } else if (!posting && was) {
      const sent = attempt.current;
      attempt.current = null;
      setPending(sent && postAccepted(sent, now) ? sent : null);
    }
  }, [posting]);
  const delivered =
    pending !== null && messages.some((message) => isDelivery(pending, message, viewerId));
  useEffect(() => {
    if (delivered) setPending(null);
  }, [delivered]);
  useEffect(() => {
    if (!pending) return;
    const timer = window.setTimeout(() => setPending(null), PENDING_POST_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [pending]);
  const current = pending && !delivered ? pending : null;

  const key = storeKey(crew.connectionId, crew.channelId);
  useLayoutEffect(() => {
    if (!current) return;
    publish(key, current);
    return () => {
      // Only this post's own entry: a newer publish for the same channel is not taken back.
      if (published.get(key) === current) publish(key, null);
    };
  }, [key, current]);
  return current;
}
