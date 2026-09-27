import { useCallback, useRef } from 'react';
import { toastWarning } from '../../../toasts';
import type { ChatCrewAccess } from './chatCrewAccess';
import { useCrewComposerHold, type CrewComposerHold } from './ChatCrewAccessBar';
import { accessCopy } from './copy';

/**
 * The Crew hold, for every path that sends from a chat without its composer (final acceptance F1).
 *
 * A chat whose Crew access was revoked, ran out or ended with a settings change holds its composer
 * (`ChatCrewAccess.blocksComposer`): the daemon refuses its turns, so the chat says why instead of
 * letting the next message fail. The composer was the only door that asked. **Edit in place** went
 * straight to the chat stream, which truncated the stored conversation at the edited message and
 * only then started a turn the daemon refused — measured twice, deleting 20 and 24 stored rows for
 * a turn that could never run. Diverge copied the chat's channel messages into a new chat that no
 * grant holds; Retry, "Send again", a queued message, a workflow activity and a steer each started
 * a turn the same way.
 *
 * Every one of them now asks this first, before anything is changed or sent. Display and
 * sequencing only: the daemon refuses the turn whatever this says.
 */

/** Say why a held chat cannot send: the hold's own words, as Enter in the composer says them. */
export function sayCrewHold(hold: CrewComposerHold | null): void {
  toastWarning({
    title: hold?.title ?? accessCopy.chatBlockedSendTitle,
    msg: hold?.message ?? accessCopy.chatBlockedReason,
  });
}

/**
 * `refuse()`: true when the chat is held — and then, unless `quiet`, the person is told why — so
 * the caller sends nothing and changes nothing. Stable across renders; it reads the latest hold.
 */
export function useCrewResendHold(
  access: Pick<ChatCrewAccess, 'sessionId' | 'blocksComposer'>
): (options?: { quiet?: boolean }) => boolean {
  const hold = useCrewComposerHold(access.sessionId);
  const latest = useRef({ held: access.blocksComposer, hold });
  latest.current = { held: access.blocksComposer, hold };
  return useCallback((options?: { quiet?: boolean }) => {
    if (!latest.current.held) return false;
    if (!options?.quiet) sayCrewHold(latest.current.hold);
    return true;
  }, []);
}
