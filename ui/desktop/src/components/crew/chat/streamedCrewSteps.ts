import { useCallback, useSyncExternalStore } from 'react';
import type { Message } from '../../../api';
import { getCrewStepContent } from '../../../types/message';

/**
 * The Crew step cards this window received on a chat's live stream, by chat.
 *
 * The daemon never stores a Crew step card (`is_ephemeral_card`): one exists only while the call
 * that parked it waits, and it reaches this window only as a streamed `Message`, either live or in
 * the turn's backlog replayed to a window that opened the chat later. So a card read back from a
 * stored transcript (History, an imported session file, a chat's saved rows) is not one the daemon
 * is waiting on, and its words are whatever that file says. A card draws its buttons only
 * when its ID is here and its chat's turn still runs (`CrewStepCard`); everywhere else it draws one
 * fixed line.
 *
 * Memory only, and per window: a reload empties it, and the turn's backlog fills it again.
 */
const LIMIT = 256;

/** `${sessionId}\0${cardId}`, oldest first. NUL never occurs in either ID. */
const streamed = new Set<string>();
const listeners = new Set<() => void>();

function keyOf(sessionId: string, id: string): string {
  return `${sessionId}\0${id}`;
}

/**
 * Note the Crew step card a streamed message carries, if it carries one. Called by the chat
 * stream for every `Message` event it applies, and never for a stored conversation.
 */
export function noteStreamedCrewStep(sessionId: string, message: Message): void {
  const content = getCrewStepContent(message);
  if (!content || content.data.actionType !== 'crewStep') return;
  const key = keyOf(sessionId, content.data.id);
  if (streamed.has(key)) return;
  streamed.add(key);
  while (streamed.size > LIMIT) {
    const oldest = streamed.values().next().value;
    if (oldest === undefined) break;
    streamed.delete(oldest);
  }
  for (const listener of listeners) listener();
}

/** Whether this window received the card `id` on chat `sessionId`'s live stream. */
export function wasStreamedCrewStep(sessionId: string, id: string): boolean {
  return streamed.has(keyOf(sessionId, id));
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** {@link wasStreamedCrewStep}, kept current for a component. */
export function useStreamedCrewStep(sessionId: string, id: string): boolean {
  const read = useCallback(() => wasStreamedCrewStep(sessionId, id), [sessionId, id]);
  return useSyncExternalStore(subscribe, read, read);
}

export function resetStreamedCrewStepsForTests(): void {
  streamed.clear();
}
