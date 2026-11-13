import type { Message } from '../api';
import { getToolConfirmationContent } from '../types/message';

/**
 * Approval cards that expired unanswered.
 *
 * When a card's answer window closes before anyone answers it, the daemon
 * stops waiting and follows the card, on the same stream, with a user-only
 * inline notice whose `expiredActionId` names it
 * (`pending_user_action::card_expiry_notice`). The card is then drawn as
 * expired, without its buttons, wherever the transcript holds both: live, in
 * another window or observer tab, in `biorouter serve`, and in a reload that
 * replays the turn. Neither the card nor the notice is ever stored, so a
 * transcript read from storage holds neither.
 */

/** The approval card this notice reports as expired, if it is such a notice. */
export function expiredApprovalId(message: Message): string | undefined {
  for (const content of message.content) {
    if (content.type === 'systemNotification' && content.expiredActionId) {
      return content.expiredActionId;
    }
  }
  return undefined;
}

/** The id of the approval card this message draws, if it draws one. */
function approvalCardId(message: Message): string | undefined {
  const content = getToolConfirmationContent(message);
  if (!content || !('id' in content.data)) return undefined;
  return typeof content.data.id === 'string' ? content.data.id : undefined;
}

/**
 * Has approval card `id`, drawn at `index`, been reported expired? The notice
 * always follows its card, so only the rows after it are read.
 */
export function approvalCardExpired(
  messages: readonly Message[],
  index: number,
  id: string
): boolean {
  for (let next = Math.max(0, index + 1); next < messages.length; next++) {
    if (expiredApprovalId(messages[next]) === id) return true;
  }
  return false;
}

/**
 * Is this an expiry notice whose card the transcript also draws? Then the card
 * itself says "Expired", and the notice adds nothing a second time. A notice
 * whose card is not there (a feed that joined after the card went by) keeps
 * its own line, the daemon's sentence naming what expired.
 */
export function expiryShownOnItsCard(messages: readonly Message[], message: Message): boolean {
  const id = expiredApprovalId(message);
  if (!id) return false;
  return messages.some((candidate) => approvalCardId(candidate) === id);
}
