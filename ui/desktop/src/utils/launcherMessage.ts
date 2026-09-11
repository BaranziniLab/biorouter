import type { NavigateFunction } from 'react-router-dom';
import { createSession } from '../sessions';
import { toastError } from '../toasts';
import { startChatFailureNotice } from './startChatFailure';
import { getInitialWorkingDir } from './workingDir';
import type { PairRouteState } from '../components/Pair';

/**
 * Deliver a LAUNCHER message to a chat tab (issue #38).
 *
 * The launcher's window arrives on `/pair?initialMessagePending=true` with its
 * message parked in the main process (see main.ts); once React is ready the
 * `set-initial-message` IPC hands it to App.tsx, which calls this: create the
 * session, then navigate so the ChatGroups deep-link inbox opens a tab for it.
 *
 * Two invariants of that navigation, both from the Codex B6 re-review:
 *
 *   1. The session id MUST ride the query string, not only location.state.
 *      /pair's inbox is useChatGroupsUrlSync, and its IN effect is gated on
 *      `searchParams.get('resumeSessionId')` — it returns early when the param
 *      is absent, BEFORE it ever reads location.state. A state-only navigation
 *      therefore created the session and stranded the message: no openTab, no
 *      tab, no submit. Identical to the Home-composer bug fixed in
 *      createNavigationHandler's 'pair' case (see navigationUtils.ts).
 *
 *   2. `replace: true` — the entry being replaced is the bootstrap
 *      `/pair?initialMessagePending=true` route main.ts loaded the window on.
 *      That marker exists only to hold the empty-pair redirect open until the
 *      IPC round-trip completes (useEmptyPairRedirect); left in history, Back
 *      would resurrect a zero-tab /pair whose marker suppresses the redirect
 *      indefinitely — a dead end nobody should be able to traverse into.
 *
 * The message itself still travels as location.state, which is where the IN
 * effect reads cargo from once the param has opened the gate. On failure we
 * navigate nowhere: the marker keeps the window parked on the empty pane (the
 * pre-#38 resting state) rather than silently discarding the launch intent —
 * and the failure is said out loud, in `startChatFailureNotice`'s words, since
 * a parked pane explains nothing by itself.
 */
export async function deliverLauncherMessage(
  navigate: NavigateFunction,
  initialMessage: string
): Promise<void> {
  try {
    const session = await createSession(getInitialWorkingDir(), {});
    const state: PairRouteState = { initialMessage };
    navigate(`/pair?resumeSessionId=${encodeURIComponent(session.id)}`, {
      replace: true,
      state,
    });
  } catch (error) {
    console.error('Failed to create session for launcher message:', error);
    toastError(startChatFailureNotice(error, { kept: false }));
  }
}
