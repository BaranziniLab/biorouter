import type { UserAttachment } from '../types/message';

/**
 * What a chat's composer has QUEUED — messages the person pressed Send on while
 * a turn was running — kept for the CHAT, so it outlives the composer that shows
 * it.
 *
 * Only a pane's active tab mounts a `BaseChat`, so clicking another tab unmounts
 * the composer, and splitting, collapsing or dragging a pane rebuilds it. The
 * queue used to be that component's state. Measured on 1.90.4 (main 1038a113),
 * desktop and `biorouter serve`, against Versa GPT-5.5:
 *
 *   • a message showing as "Next" behind a running turn was gone after clicking
 *     another tab and back; the turn ended and it was never sent, nor put back
 *     in the composer. The unmount recovery kept only offers already in flight;
 *   • and it kept those AS QUEUE ROWS. A drained message's submit does not
 *     answer until the turn it started ends, so a tab switch during that turn
 *     brought the message back as "Next" while it was running, and the turn's
 *     end sent it a SECOND time (serve, production bundle: two identical user
 *     messages, two answers). The same steps in the dev app showed no row, so
 *     check this in a production bundle, not only under `vite` dev.
 *
 * SO THERE ARE TWO STATES, AND A MESSAGE IS IN EXACTLY ONE:
 *   • PARKED — waiting to be sent. The composer parks its queue here when it
 *     unmounts, and the next composer for the same chat claims it when it
 *     mounts; it is then an ordinary queue row again.
 *   • IN FLIGHT — handed to a submit that has not answered. It belongs to that
 *     submit and is never shown or drained by another composer. If the submit
 *     answers "not taken" after its composer is gone, the message comes back
 *     here (to the chat's mounted composer if there is one, else parked at the
 *     head). If it is taken, there is nothing to do.
 *
 * WHEN A CLAIMED QUEUE IS SENT. A queue parked while its turn was still running
 * (or mid-drain) was due to go at that turn's end. If the turn ended while no
 * composer was mounted, nobody could send it then — the submit path, with its
 * continuation-ownership, workflow and read-only gates, is the mounted
 * composer's, and sending from anywhere else would bypass them. So the composer
 * that claims it drains it exactly as the turn's end would have: the head goes,
 * through the same submit, as soon as that composer is mounted and the chat is
 * idle; the rest waits for the turn the head starts. A queue that was already
 * sitting idle when it was parked (left visible after a refusal, or paused)
 * comes back as it was, and is not sent by itself.
 *
 * WHY IT CANNOT LEAK: a key names one chat, and a composer claims only its own
 * chat's key. A composer with no chat (Home, a new tab before its start answers)
 * has no key and parks nothing here: it has no chat to send to later, so its
 * queue is handed back to its own draft instead (`utils/composerDrafts.ts`).
 * Another window is another renderer and never sees this map.
 *
 * LIFETIME — definite events, never a clock:
 *   • claimed by the next composer mounted for the chat;
 *   • `ChatGroupsProvider` RETAINS only the queues of chats open in a tab of
 *     this window. Closing the tab drops its queue and deletes the temp images
 *     it owned, so reopening the chat later never sends a stale message;
 *   • it is renderer memory. A RELOAD drops it, deliberately, for the same
 *     reasons a draft does not survive one: a staged image is a temp file
 *     nothing promises to keep, and writing unsent message text to disk is a
 *     privacy decision this does not get to make on the way past.
 *
 * WHY IT CANNOT GROW: at most one entry per chat open in a tab, each holding
 * what that chat's composer had queued.
 */

export interface QueuedMessage {
  id: string;
  content: string;
  attachments?: UserAttachment[];
  /** Renderer-owned temp images to unlink if the queue discards this message.
   * Kept separate from `attachments`: that array may also contain a user's
   * original file path, which must never be deleted. */
  ownedTempAttachmentPaths?: string[];
  timestamp: number;
}

export type ParkedComposerQueue = {
  /** In queue order; the head is sent first. */
  messages: QueuedMessage[];
  paused: boolean;
  interruption: string | null;
  /** Due to be sent at a turn's end that no composer was mounted to see. */
  sendWhenIdle: boolean;
};

const CHAT_KEY_PREFIX = 'chat:';

/** The key a chat's queue is kept under; `null` for a composer with no chat. */
export function composerQueueKey(sessionId: string | null | undefined): string | null {
  return sessionId ? `${CHAT_KEY_PREFIX}${sessionId}` : null;
}

const parked = new Map<string, ParkedComposerQueue>();
/** Message ids handed to a submit that has not answered, per chat key. */
const inFlight = new Map<string, Set<string>>();
const returnListeners = new Map<string, Set<(message: QueuedMessage) => void>>();

function withoutDuplicates(messages: readonly QueuedMessage[]): QueuedMessage[] {
  const seen = new Set<string>();
  return messages.filter((message) => {
    if (seen.has(message.id)) return false;
    seen.add(message.id);
    return true;
  });
}

export function deleteOwnedTempAttachments(messages: readonly QueuedMessage[]): void {
  const ownedPaths = new Set(messages.flatMap((message) => message.ownedTempAttachmentPaths ?? []));
  for (const path of ownedPaths) window.electron?.deleteTempFile(path);
}

/**
 * A composer is going away holding this queue. Anything already parked under
 * the key (a message handed back while no composer was mounted) stays ahead of
 * it. An empty queue parks nothing.
 */
export function parkComposerQueue(key: string, queue: ParkedComposerQueue): void {
  const existing = parked.get(key);
  const messages = withoutDuplicates([...(existing?.messages ?? []), ...queue.messages]);
  if (messages.length === 0) {
    parked.delete(key);
    return;
  }
  parked.set(key, {
    messages,
    paused: queue.paused,
    interruption: queue.interruption ?? existing?.interruption ?? null,
    sendWhenIdle: queue.sendWhenIdle || Boolean(existing?.sendWhenIdle),
  });
}

/** Take the chat's parked queue, if any. It is the claimer's from here on. */
export function claimComposerQueue(key: string): ParkedComposerQueue | undefined {
  const queue = parked.get(key);
  parked.delete(key);
  return queue;
}

export function readParkedComposerQueue(key: string): ParkedComposerQueue | undefined {
  return parked.get(key);
}

export function beginQueuedOffer(key: string, messageId: string): void {
  const ids = inFlight.get(key) ?? new Set<string>();
  ids.add(messageId);
  inFlight.set(key, ids);
}

export function endQueuedOffer(key: string, messageId: string): void {
  const ids = inFlight.get(key);
  ids?.delete(messageId);
  if (ids?.size === 0) inFlight.delete(key);
}

/** Is a message of this chat handed to a submit that has not answered yet? */
export function hasQueuedOfferInFlight(key: string): boolean {
  return (inFlight.get(key)?.size ?? 0) > 0;
}

/**
 * A submit answered "not taken" for a message whose composer is gone. It goes
 * to the composer mounted for the chat now, or waits at the head of the chat's
 * parked queue, due to be sent when a composer is next mounted and idle.
 */
export function returnQueuedOffer(key: string, message: QueuedMessage): void {
  const listeners = [...(returnListeners.get(key) ?? [])];
  const current = listeners[listeners.length - 1];
  if (current) {
    current(message);
    return;
  }
  const existing = parked.get(key);
  parked.set(key, {
    messages: withoutDuplicates([message, ...(existing?.messages ?? [])]),
    paused: existing?.paused ?? false,
    interruption: existing?.interruption ?? null,
    sendWhenIdle: true,
  });
}

/** A mounted composer receives the chat's messages handed back while it is mounted. */
export function subscribeQueuedOfferReturns(
  key: string,
  listener: (message: QueuedMessage) => void
): () => void {
  const listeners = returnListeners.get(key) ?? new Set();
  listeners.add(listener);
  returnListeners.set(key, listeners);
  return () => {
    const current = returnListeners.get(key);
    current?.delete(listener);
    if (current?.size === 0) returnListeners.delete(key);
  };
}

/**
 * Keep the parked queues of these chats — the ones open in a tab of this
 * window — and no other. A queue whose chat is no longer open anywhere is gone
 * for good: dropped, with the temp images it owned.
 */
export function retainComposerQueues(openSessionIds: Iterable<string>): void {
  const live = new Set<string>();
  for (const sessionId of openSessionIds) {
    const key = composerQueueKey(sessionId);
    if (key) live.add(key);
  }
  for (const [key, queue] of [...parked]) {
    if (live.has(key)) continue;
    parked.delete(key);
    deleteOwnedTempAttachments(queue.messages);
  }
}

export function resetComposerQueuesForTests(): void {
  parked.clear();
  inFlight.clear();
  returnListeners.clear();
}
