import type { UserAttachment } from '../types/message';

/**
 * `restore-chat-input`: giving a message back to the composer of an EXISTING
 * chat, addressed by that chat's id.
 *
 * It is a plain window event — no buffering, delivered to whichever composers
 * are listening at that instant — and that is only safe because a chat id names
 * one chat. So it is now refused for anything else, on both ends:
 * `restoreComposerText` will not send one without a chat id, and
 * `composerRestoreIsFor` will not let a composer take one that does not name
 * the composer's own chat.
 *
 * ⚠ A NEW chat has no id, and this channel used to address it as `''` (and Home's
 * as `null`). Every mounted fresh-tab composer matched `'' === ''`, so a start
 * that failed in one pane REPLACED the unsent text in every other pane's new
 * tab — measured in the dev app on 1.90.4, 2 of 2, and shipped since #303's
 * parent. A new chat's composer is addressed by its TAB instead, through
 * `utils/composerDrafts.ts`, and a composer's own refused send is handed back
 * to itself; neither goes near this event.
 *
 * Today's one sender is `BaseChat.returnInitialMessageToComposer`: a message
 * that arrived as a chat's route cargo and was refused by that chat.
 */
export type ComposerRestore = {
  /** The chat the message belongs to. Required, and never empty. */
  sessionId: string;
  value: string;
  attachments?: UserAttachment[];
};

export const RESTORE_CHAT_INPUT_EVENT = 'restore-chat-input';

/** Hand `detail` to the composer of the chat it names, if one is listening. */
export function restoreComposerText(detail: ComposerRestore): void {
  if (typeof detail.sessionId !== 'string' || !detail.sessionId) return;
  window.dispatchEvent(new CustomEvent(RESTORE_CHAT_INPUT_EVENT, { detail }));
}

/**
 * Whether a composer for `sessionId` may take this restore. Only a composer of
 * a real chat, and only for that chat — so no new chat's composer, and no Home
 * composer, can ever be reached by a broadcast.
 */
export function composerRestoreIsFor(
  detail: unknown,
  sessionId: string | null | undefined
): detail is ComposerRestore {
  if (typeof sessionId !== 'string' || !sessionId) return false;
  if (!detail || typeof detail !== 'object') return false;
  const candidate = detail as Partial<ComposerRestore>;
  if (candidate.sessionId !== sessionId) return false;
  const hasText = typeof candidate.value === 'string' && candidate.value.length > 0;
  const hasAttachments =
    Array.isArray(candidate.attachments) && candidate.attachments.some((a) => a?.path);
  return hasText || hasAttachments;
}
