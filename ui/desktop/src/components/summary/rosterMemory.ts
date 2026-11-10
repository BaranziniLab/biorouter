import type { SessionSubagents } from '../../utils/sessionSubagents';

/**
 * The last roster the Chat summary drew for each chat, for the moments its read
 * has not answered yet or failed (H-12, live QA 2026-11-10: after a tab switch
 * the rail once showed no Phases block until the next visit). A read that
 * answers replaces it; an answer of "no subagents and no plan" forgets it.
 *
 * Process memory only, bounded, and never a source of truth: what is drawn from
 * it is the chat's own last answer, shown while a fresher one is on its way.
 */
const LIMIT = 32;
const rosters = new Map<string, SessionSubagents>();

export function rememberRoster(sessionId: string, roster: SessionSubagents): void {
  rosters.delete(sessionId);
  rosters.set(sessionId, roster);
  while (rosters.size > LIMIT) {
    const oldest = rosters.keys().next().value;
    if (oldest === undefined) break;
    rosters.delete(oldest);
  }
}

export function rememberedRoster(sessionId: string): SessionSubagents | null {
  return rosters.get(sessionId) ?? null;
}

export function forgetRoster(sessionId: string): void {
  rosters.delete(sessionId);
}

/** Tests only. */
export function clearRosterMemoryForTests(): void {
  rosters.clear();
}
