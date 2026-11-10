import type { Message } from '../../api';
import { usageLimitNoticeKind } from './usageLimitNotice';

/** How far back from the end a notice is looked for: it is written as a turn ends. */
const LOOK_BACK = 12;

/**
 * A key for the newest stored usage limit notice near the end of the transcript (its id, or its
 * place when it has none), or `undefined` when there is none. `BaseChat` hands it to `ChatInput`,
 * which reads the chat's limit again when it changes. It never decides that the chat is paused:
 * an old notice stays in the transcript after the limit is raised or lifted.
 */
export function latestUsageLimitNoticeKey(messages: readonly Message[]): string | undefined {
  const stop = Math.max(0, messages.length - LOOK_BACK);
  for (let index = messages.length - 1; index >= stop; index--) {
    const message = messages[index];
    if (usageLimitNoticeKind(message) !== null) return message.id ?? `at:${index}`;
  }
  return undefined;
}
