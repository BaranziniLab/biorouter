import type { UserAttachment } from '../types/message';

/**
 * Giving a message back to the composer when the chat it was typed into never
 * started.
 *
 * `restore-chat-input` is a plain window event with no buffering: it reaches
 * whichever composer is listening at the instant it is dispatched, and nobody
 * else, ever. That is enough for a composer that is alive and listening, and it
 * is NOT enough for the fresh tab's, which the failure itself replaces —
 * `BaseChat` renders its composer in two places (the centred empty state and
 * the bar under a transcript) and `isCreatingSession` moves it between them, so
 * a failed start REMOUNTS it and the instance the event reached is discarded
 * before it ever paints.
 *
 * THE DURABLE COPY IS NOT HERE. It is `BaseChat`'s own `keptMessage` state,
 * handed to whatever composer that surface renders next. This module keeps only
 * the broadcast, for the composers the broadcast was written for.
 *
 * WHY NOT HERE — measured, not assumed. #303 put the message in a module-level
 * map keyed by chat and deleted it on a `setTimeout(..., 0)`: a lifetime that
 * had to beat the very remount it existed to survive. It won that race exactly
 * when the failure also rendered a NEW toast node, and error toasts are
 * `autoClose: false` and deduped by their own text, so a second identical
 * failure renders no toast at all — and pressing the same failing send again is
 * what a person does. In the dev app on 1.90.4 (2026-09-12): first failure, new
 * toast node, remount +38 ms after the dispatch -> kept; press Enter again, no
 * new toast node, remount +43 ms -> the composer was EMPTY while the first
 * toast was still on screen reading "Your message was kept." 3 of 3 sends that
 * rendered no new toast node lost the message; 2 of 2 that rendered one kept it.
 *
 * A timer was not the only thing wrong with parking it here. A pre-session
 * composer has no chat id — it is addressed as `''` by one caller and `null` by
 * another — so a store keyed by chat cannot tell the fresh tab in one pane from
 * the fresh tab in another, or from Home's composer. Surface-owned state can:
 * it is reachable only from the one component instance that holds it.
 */
export type ComposerRestore = {
  /** The chat the message was typed into; `null`/`''` is a chat not yet created. */
  sessionId: string | null;
  value: string;
  attachments?: UserAttachment[];
};

export const RESTORE_CHAT_INPUT_EVENT = 'restore-chat-input';

/**
 * Hand `detail.value` to the composer for `detail.sessionId` that is listening
 * right now. A composer the same failure is about to replace is NOT one of
 * them; its replacement is served by the surface's own copy.
 */
export function restoreComposerText(detail: ComposerRestore): void {
  window.dispatchEvent(new CustomEvent(RESTORE_CHAT_INPUT_EVENT, { detail }));
}
