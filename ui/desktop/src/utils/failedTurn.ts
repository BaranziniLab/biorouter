import type { Message } from '../api';
import { generateMessageId } from '../types/message';

/**
 * A user row that opens a turn, or steers one: shown to the person and
 * carrying more than tool results. A tool result is a user row too, and a
 * model-only note (a retry hint) is one the person never sees; neither starts
 * a turn.
 */
export function opensTurn(message: Message): boolean {
  return (
    message.role === 'user' &&
    message.metadata?.userVisible !== false &&
    message.content.some((content) => content.type !== 'toolResponse')
  );
}

/**
 * The prompt Retry sends again after the daemon ended a turn with a provider
 * failure and stored rows after that prompt: its "Ran into this error" notice,
 * and anything that landed with it (a decision monitor's marker). Null when the
 * turn's opening row is not something the person wrote and could send again.
 *
 * Retry used to re-send only a TRAILING user row, so a failure the daemon had
 * already answered with a stored notice left Retry with nothing to send: it
 * cleared the error and did nothing else (live QA, 2026-11-11).
 */
export function failedTurnPrompt(messages: Message[]): Message | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (!opensTurn(message)) continue;
    // Sent by another agent, or the context a spawn started with: not the
    // person's words, so not theirs to send again under their name.
    const kind = message.metadata?.provenance?.kind;
    if (kind === 'agent_injection' || kind === 'spawn_context') return null;
    const resendable =
      message.content.length > 0 &&
      message.content.every((content) => content.type === 'text' || content.type === 'image') &&
      message.content.some(
        (content) =>
          content.type === 'image' || (content.type === 'text' && content.text.trim() !== '')
      );
    return resendable ? message : null;
  }
  return null;
}

/**
 * The prompt as a new row: the same words and images under a new id. Not the
 * old row again, which the daemon would take as a replay of a row it already
 * holds and store nothing new, leaving the model to answer its own error notice.
 */
export function promptToSendAgain(prompt: Message): Message {
  return {
    id: generateMessageId(),
    role: 'user',
    created: Math.floor(Date.now() / 1000),
    content: prompt.content,
    metadata: { userVisible: true, agentVisible: true },
  };
}
