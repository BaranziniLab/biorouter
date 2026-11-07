import { useCallback, useSyncExternalStore } from 'react';
import type { Message } from '../../../api';
import { getCrewStepContent } from '../../../types/message';

/**
 * The life of the Crew step cards in this window, by chat: which ones it received on a chat's
 * live stream, and which of them the daemon has said ended.
 *
 * The daemon never stores a Crew step card that waits (`is_ephemeral_card`): one exists only
 * while the call that parked it waits, and it reaches this window only as a streamed `Message`,
 * either live or in the turn's backlog replayed to a window that opened the chat later (the
 * daemon replays only the cards still waiting). So a card read back from a stored transcript
 * (History, an imported session file, a chat's saved rows) is not one the daemon is waiting on,
 * and its words are whatever that file says. A card draws its buttons only when its ID is here
 * and the daemon has not said it ended (`CrewStepCard`); everywhere else it draws one fixed line.
 *
 * The end (WI-17) is the daemon's `ActionRequiredEnded` frame: the card's ID and how it ended,
 * never a value. It is noted for any card ID, before or after its card arrives, since the daemon
 * may say a card ended before the chat's loop drew it.
 *
 * Memory only, and per window: a reload empties it, and the turn's backlog fills it again.
 */
const LIMIT = 256;

/** How the daemon said a card ended (`ActionRequiredEnding` on the wire). */
export type CardEnding = 'answered' | 'dismissed' | 'timed_out' | 'withdrawn';

/** `${sessionId}\0${cardId}`, oldest first. NUL never occurs in either ID. */
const streamed = new Set<string>();
/** `${sessionId}\0${cardId}` to how it ended, oldest first. */
const ended = new Map<string, CardEnding>();
const listeners = new Set<() => void>();

function keyOf(sessionId: string, id: string): string {
  return `${sessionId}\0${id}`;
}

function notify(): void {
  for (const listener of listeners) listener();
}

/** Keep `kept` to {@link LIMIT} entries, dropping the oldest. */
function trim(kept: Set<string> | Map<string, CardEnding>): void {
  while (kept.size > LIMIT) {
    const oldest = kept.keys().next().value;
    if (oldest === undefined) break;
    kept.delete(oldest);
  }
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
  trim(streamed);
  notify();
}

/** Whether this window received the card `id` on chat `sessionId`'s live stream. */
export function wasStreamedCrewStep(sessionId: string, id: string): boolean {
  return streamed.has(keyOf(sessionId, id));
}

/** A wire value read as one of the four endings; anything else still means the card ended. */
function endingOf(value: unknown): CardEnding {
  return value === 'answered' || value === 'dismissed' || value === 'timed_out'
    ? value
    : 'withdrawn';
}

/**
 * Note that the daemon said the card `id` in chat `sessionId` ended. Called by the chat stream
 * for every `ActionRequiredEnded` frame, whatever kind of card it names.
 */
export function noteCardEnded(sessionId: string, id: string, ending: unknown): void {
  const key = keyOf(sessionId, id);
  if (ended.has(key)) return;
  ended.set(key, endingOf(ending));
  trim(ended);
  notify();
}

/** How the card `id` in chat `sessionId` ended, or `undefined` while the daemon has not said. */
export function cardEndingOf(sessionId: string, id: string): CardEnding | undefined {
  return ended.get(keyOf(sessionId, id));
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

/** {@link cardEndingOf}, kept current for a component. */
export function useCardEnding(sessionId: string, id: string): CardEnding | undefined {
  const read = useCallback(() => cardEndingOf(sessionId, id), [sessionId, id]);
  return useSyncExternalStore(subscribe, read, read);
}

export function resetStreamedCrewStepsForTests(): void {
  streamed.clear();
  ended.clear();
}
