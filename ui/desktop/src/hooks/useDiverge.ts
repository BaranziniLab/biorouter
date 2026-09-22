import { useCallback } from 'react';
import { divergeSession } from '../api';
import { toastError } from '../toasts';
import { notifySessionListChanged } from '../utils/sessionListCache';
import { isPrivateCopyRefusal, userActionHeaders } from '../utils/userAction';

/**
 * Issue #56 DR-19, the user's half of the refusal.
 *
 * The daemon's body is model-facing prose ("Do not retry; the same call will be
 * refused again") and is the wrong thing to put in front of a person, but the
 * generic fallback was worse: it named neither the cause nor the way out. This
 * pair says what happened and what to do, and — like the model picker's
 * equivalent — names the backend rather than accusing the person at the keyboard
 * of being a model, because that is the only way this is reached.
 *
 * The user should ordinarily never see it: this surface carries the proof. It
 * appears on a backend the user started themselves (open question 23), and, far
 * more rarely, when the source chat was ratcheted private by a turn running
 * while the branch was being made.
 */
export const PRIVATE_COPY_TOAST_TITLE = "Can't diverge this private chat";
export const PRIVATE_COPY_TOAST_MSG =
  'Diverging a private chat creates another chat on the same private model, so only you can ' +
  'do it, and this backend could not confirm the request came from you. No new chat was created ' +
  'and this chat is unchanged. If you started the backend yourself, diverge from a Biorouter ' +
  'window instead.';

export interface UseDivergeResult {
  /**
   * Branch the given session into a brand-new session that inherits the
   * conversation history up to the last complete assistant answer, leaving the
   * original untouched.
   *
   * The branch always opens in a NEW focused Electron window, leaving the
   * current window exactly where it is.
   *
   * `truncateAfterMs` and `truncateAfterId` identify the assistant message a
   * per-message Diverge button was clicked on; the durable id takes precedence
   * and the timestamp remains as a compatibility fallback. Omit both to branch
   * from the most recent complete answer.
   *
   * Returns the new session id on success, or null if it failed (a toast is
   * shown on failure).
   */
  diverge: (
    sessionId: string,
    truncateAfterMs?: number,
    truncateAfterId?: string
  ) => Promise<string | null>;
}

export function useDiverge(): UseDivergeResult {
  const diverge = useCallback(
    async (
      sessionId: string,
      truncateAfterMs?: number,
      truncateAfterId?: string
    ): Promise<string | null> => {
      if (!sessionId) {
        toastError({ title: 'Diverge failed', msg: 'No open chat to diverge from.' });
        return null;
      }
      try {
        const response = await divergeSession({
          path: { session_id: sessionId },
          body: {
            ...(truncateAfterMs != null ? { truncateAfter: truncateAfterMs } : {}),
            ...(truncateAfterId ? { truncateAfterId } : {}),
          },
          // Issue #56 DR-19: the branch inherits the source chat's provider, so
          // branching a PRIVATE chat mints a new private-capability session and
          // the daemon refuses it without proof the request came from the
          // person at the keyboard. This is that surface — every call it makes
          // is a user act — so it carries Task 18A's header. Branching a public
          // chat is unaffected either way.
          headers: await userActionHeaders(),
          throwOnError: true,
        });

        const newSessionId = response.data?.sessionId;
        const workingDir = response.data?.workingDir;
        // The backend resolves and LOCKS the canonical branch name (e.g.
        // "Foo (branch 1)") and hands it back here. Thread it into the new
        // window so the branch's tab is born with the real name instead of the
        // "New chat" placeholder — which is what made the tab name and the
        // Recents name disagree for a diverged session.
        const branchName = response.data?.name;
        if (!newSessionId) {
          throw new Error('Diverge did not return a new session ID');
        }

        // The branch is a brand-new session created purely over HTTP; it fires
        // no `session-created` window event, and those are per-renderer anyway.
        // Announce the membership change so every window's Recents / See-all /
        // Home picks up the branch without waiting for an unrelated turn.
        notifySessionListChanged();

        // Open a NEW focused Electron window for the branch. The current
        // window is never navigated or changed.
        window.electron.createDivergedChatWindow(workingDir, newSessionId, branchName);
        return newSessionId;
      } catch (err) {
        console.error('Failed to diverge session:', err);
        // Issue #56 DR-19 before the generic arm. Under `throwOnError` the
        // generated client throws the PARSED BODY, so a refusal arrives here as
        // a plain string — which is not an `Error`, so the fallback below used
        // to answer for it and the daemon's whole explanation was discarded.
        if (isPrivateCopyRefusal(err)) {
          toastError({
            title: PRIVATE_COPY_TOAST_TITLE,
            msg: PRIVATE_COPY_TOAST_MSG,
            traceback: String(err),
          });
          return null;
        }
        toastError({
          title: 'Diverge failed',
          // A thrown string is the daemon's own body and says more than the
          // fallback ever could; the fallback is for a thrown value that is
          // neither an `Error` nor text.
          msg:
            err instanceof Error
              ? err.message
              : typeof err === 'string' && err.trim()
                ? err
                : 'Could not diverge this chat.',
        });
        return null;
      }
    },
    []
  );

  return { diverge };
}
