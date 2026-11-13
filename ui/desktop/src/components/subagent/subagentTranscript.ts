import type { Message } from '../../api';

/**
 * The prompt the daemon starts every delegated subagent with
 * (`subagent_handler.rs`: `workflow.prompt.unwrap_or("Begin.")`). The task
 * itself rides in the child's instructions, which the tab's header shows as
 * "Instructions".
 */
export const SUBAGENT_START_PROMPT = 'Begin.';

function textOf(message: Message): string | null {
  let text = '';
  for (const part of message.content) {
    if (part.type !== 'text') return null;
    text += part.text;
  }
  return text;
}

/**
 * A subagent's transcript as its tab draws it.
 *
 * Two records the spawn writes are dropped from the view, never from the chat
 * (its saved rows and exports keep both):
 *
 * - the spawn record (`provenance.kind === 'spawn_context'`), whose one place is
 *   the header's "Instructions" (F-15);
 * - the synthetic first prompt, "Begin." (R2-4, live QA 2026-11-10). The person
 *   did not write it, yet it drew as their message, with an "Edit message:
 *   Begin." pencil whose edit would have re-run the child from a prompt that
 *   says nothing. Only the exact prompt is dropped: when steering a person sent
 *   before the run started rode into it, the message carries their words and
 *   stays.
 */
export function subagentTranscript(messages: Message[]): Message[] {
  let firstPromptSeen = false;
  return messages.filter((message) => {
    if (message.metadata?.provenance?.kind === 'spawn_context') return false;
    if (firstPromptSeen || message.role !== 'user') return true;
    firstPromptSeen = true;
    return textOf(message)?.trim() !== SUBAGENT_START_PROMPT;
  });
}
