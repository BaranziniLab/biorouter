import type { UserAttachment } from '../types/message';

/**
 * Giving a message back to the composer when the chat it was typed into never
 * started.
 *
 * `restore-chat-input` is a plain window event with no buffering: it reaches
 * whichever composer is listening at the instant it is dispatched, and nobody
 * else, ever. That was enough for the surfaces it was written for and NOT
 * enough for the one that matters most - the fresh tab's composer.
 *
 * Measured in the dev app on 1.90.4 (2026-09-12) with `POST /agent/start`
 * answering 500 from a fresh tab: the toast appeared, the composer stayed on
 * "New chat", and the box was empty. `BaseChat` renders its composer in two
 * places - the centred empty state and the bar under a transcript - and
 * `isCreatingSession` moves it between them, so a failed start REMOUNTS it. The
 * instance the event reached was discarded before it ever painted.
 *
 * So the message is also PARKED here, and a composer reads what was left for it
 * when it mounts. The event is still dispatched exactly as before - a composer
 * that is alive and listening restores from it as it always did - so this can
 * only add a rescue, never remove one.
 *
 * ⚠ Reading does NOT consume. Measured on the same failure, the composer is
 * rebuilt TWICE in the 20 ms after the park (mounts at +19 ms and +21 ms), and
 * a park the first mount consumed left the second - the one on screen - empty,
 * which is the bug wearing a different hat. The park is removed by time
 * instead: one task, which every mount the failure itself causes lands inside,
 * and which is far too short for the message to reappear in some unrelated
 * "New chat" later. If that expiry ever did win the race, the behaviour would
 * be exactly what it is today - the event alone - so the worst case is the
 * status quo.
 */
export type ComposerRestore = {
  /** The chat the message was typed into; `null`/`''` is a chat not yet created. */
  sessionId: string | null;
  value: string;
  attachments?: UserAttachment[];
};

export const RESTORE_CHAT_INPUT_EVENT = 'restore-chat-input';

/** A pre-session composer is addressed as `''` by one caller and `null` by another. */
const keyOf = (sessionId: string | null | undefined): string => sessionId ?? '';

const parked = new Map<string, ComposerRestore>();

/**
 * Hand `detail.value` back to the composer for `detail.sessionId`: to the one
 * listening now, and to whichever composer takes its place if it is replaced.
 */
export function restoreComposerText(detail: ComposerRestore): void {
  const key = keyOf(detail.sessionId);
  parked.set(key, detail);
  window.dispatchEvent(new CustomEvent(RESTORE_CHAT_INPUT_EVENT, { detail }));
  setTimeout(() => {
    // Identity, not presence: a second failure while this one is still parked
    // owns the key, and clearing it blindly would drop the newer message.
    if (parked.get(key) === detail) parked.delete(key);
  }, 0);
}

/**
 * The message parked for this chat's composer, if a start has just failed for
 * it. Reading leaves it parked; see the note above.
 */
export function parkedComposerRestore(
  sessionId: string | null | undefined
): ComposerRestore | undefined {
  return parked.get(keyOf(sessionId));
}
