import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react';
import type { CrewMessage } from '../crewApi';
import { lastPostOutcome } from '../state/crewSend';
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
  /**
   * The message the broker answered with, when it named one. A resend under the same key is
   * answered with the message already on screen (QA R-4), which is then delivered at once.
   */
  messageId?: string | null;
}

/**
 * How long a "Sending…" row waits for the observer to deliver its message
 * before it goes quietly. The broker accepted the post, so the message is
 * coming; this only keeps a stalled observation from leaving the row for good.
 */
export const PENDING_POST_TIMEOUT_MS = 30_000;

/**
 * The draft as a send began: what to recognize the delivered message by, and the connection and
 * channel it went to (`key`). A send is pending only in its own channel (RENDERER-4), so opening
 * another channel ends "posting" here without any answer; such an attempt is dropped.
 */
type PostAttempt = PendingPost & { key: string };

/**
 * Whether `message` is the post on its way: the message the broker answered with, by its ID, even
 * one already on screen (a deduplicated resend, QA R-4); otherwise, for an answer that named none,
 * the viewer's own words among messages that were not on screen when it was sent.
 */
function isDelivery(post: PendingPost, message: CrewMessage, viewerId: string | null): boolean {
  if (post.messageId) return message.id === post.messageId;
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
 * The post on its way from this timeline's composer, matched on arrival by the message ID the
 * broker answered with (or, when it named none, by who posted it and its words among messages that
 * were not already on screen). Published for the Files tab under the controller's connection and
 * channel while it lasts.
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
  const key = storeKey(crew.connectionId, crew.channelId);
  const [pending, setPending] = useState<PostAttempt | null>(null);
  const attempt = useRef<PostAttempt | null>(null);
  const wasPosting = useRef(false);
  const latest = useRef({ crew, messages, key });
  latest.current = { crew, messages, key };
  useLayoutEffect(() => {
    const was = wasPosting.current;
    wasPosting.current = posting;
    const { crew: now, messages: list, key: here } = latest.current;
    // A post needs something to send. An empty draft that reads "posting" did not send it: that
    // post came from a Crew screen closed since, and is on its way still (RENDERER-4).
    const hasContent =
      Boolean(now.draft.body.trim()) ||
      now.draft.attachments.length > 0 ||
      now.draft.references.length > 0;
    if (posting && !was && !hasContent) {
      attempt.current = null;
    } else if (posting && !was) {
      attempt.current = {
        key: here,
        body: now.draft.body,
        attachments: now.draft.attachments.map((file) => ({ id: file.id, name: file.name })),
        before: new Set(list.map((message) => message.id)),
      };
    } else if (!posting && was) {
      const sent = attempt.current;
      attempt.current = null;
      // The send says what became of it (`lastPostOutcome`): only a post the broker took, and
      // whose words this composer let go, stands in until it lands. Another channel's composer
      // says nothing about this post (RENDERER-4).
      const outcome =
        sent && sent.key === here ? lastPostOutcome(now.connectionId, now.channelId) : null;
      setPending(
        sent && outcome?.kind === 'accepted' ? { ...sent, messageId: outcome.messageId } : null
      );
    }
  }, [posting]);
  // A post is drawn, and delivered, only in the channel it went to (RENDERER-4): the timeline stays
  // mounted when the person opens another channel.
  const shown = pending !== null && pending.key === key ? pending : null;
  const delivered =
    shown !== null && messages.some((message) => isDelivery(shown, message, viewerId));
  useEffect(() => {
    if (delivered) setPending(null);
  }, [delivered]);
  useEffect(() => {
    if (!pending) return;
    const timer = window.setTimeout(() => setPending(null), PENDING_POST_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [pending]);
  const current = shown && !delivered ? shown : null;

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
