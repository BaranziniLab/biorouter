/**
 * Shared constants and utilities for extension error handling
 */

import { ExtensionLoadResult } from '../api/types.gen';
import { toastService, ExtensionLoadingStatus } from '../toasts';
import { isCapabilityExtension } from '../components/settings/capabilities/capabilities';
import {
  markExtensionLoadFailuresAnnounced,
  recordExtensionLoadResults,
} from './extensionLoadFailures';

export const MAX_ERROR_MESSAGE_LENGTH = 70;

// ⚠ **There is deliberately no second "is this built-in?" predicate here.**
//
// One lived at this spot, reading `bundled-extensions.json` for entries typed
// `builtin` — seven of them — while the composer's extension menu asked the
// capability catalog, which has twelve. Two answers to one question, five
// extensions apart, on two surfaces the user reads against each other. The
// catalog (`isCapabilityExtension`) is the only definition now, and it lives
// with the list the menu renders, so a capability added there cannot silently
// go on being counted here as something the user installed.

/**
 * Creates recovery hints for the "Ask biorouter" feature when extension loading fails
 */
export function createExtensionRecoverHints(errorMsg: string): string {
  return (
    `Explain the following error: ${errorMsg}. ` +
    'This happened while trying to install an extension. Look out for issues where the ' +
    "extension attempted to execute something incorrectly, didn't exist, or there was trouble with " +
    'the network configuration - VPNs like WARP often cause issues.'
  );
}

/**
 * Formats an error message for display, truncating long messages with a fallback
 * @param errorMsg - The full error message
 * @param fallback - The fallback message to show if the error is too long
 * @returns The formatted error message
 */
export function formatExtensionErrorMessage(
  errorMsg: string,
  fallback: string = 'Failed to add extension'
): string {
  return errorMsg.length < MAX_ERROR_MESSAGE_LENGTH ? errorMsg : fallback;
}

/**
 * Which chat the user is actually looking at, or null when nobody has claimed
 * focus (single-chat hosts never register). `null` means
 * "permissive" — a host that does not report focus still gets its toasts.
 */
let focusedChatSessionId: string | null = null;

/** True while the extension toast currently on screen is reporting a failure. */
let extensionToastIsFailure = false;

/**
 * Called by the chat shell as the active group changes. Readiness is only news
 * for the chat you are reading; see `showExtensionLoadResults`.
 */
export function setFocusedChatSession(sessionId: string | null): void {
  focusedChatSessionId = sessionId;
}

/** Test seam — module state would otherwise leak between cases. */
export function resetExtensionToastState(): void {
  focusedChatSessionId = null;
  extensionToastIsFailure = false;
}

/**
 * Shows toast notifications for extension load results.
 * Uses grouped toast for multiple extensions, individual error toast for single failed extension.
 *
 * Progressive conversation loading made this multi-tab-sensitive: extensions are
 * per-session, so four split groups each finish their own load and each would
 * have something to say. Two rules keep that to one honest toast:
 *
 *  1. **A clean load only toasts for the chat you are focused on.** "Everything
 *     worked" in a pane you are not reading is not news, and four of them
 *     stacking over the transcript you ARE reading is a regression. Background
 *     successes are dropped silently.
 *  2. **A failure always toasts, whichever chat it came from**, and never gets
 *     overwritten by a later success. A broken extension resurfaces later as a
 *     broken tool call, so it must not be swallowed — and since the grouped
 *     toast reuses one toast id, four chats hitting the same broken extension
 *     coalesce into one toast rather than stacking four.
 *  3. **…but a failure is announced ONCE.** The only caller is `/agent/resume`,
 *     which every renderer load performs, so rule 2 on its own re-announced the
 *     same failure after every reload — a notification for something the user
 *     did not do. `extensionLoadFailures` keeps the standing record across
 *     reloads and answers what is actually new; anything the user has already
 *     been shown is recorded silently and read on the Extensions page instead.
 *
 * @param results - Array of extension load results from the backend
 * @param sessionId - The chat these results belong to. Omit for non-chat callers
 *                    (settings, manual extension add), which always toast.
 */
export function showExtensionLoadResults(
  allResults: ExtensionLoadResult[] | null | undefined,
  sessionId?: string
): void {
  if (!allResults || allResults.length === 0) {
    return;
  }

  // ⚠ **The toast's denominator and the composer's chip must count the same
  // population, because the user reads them against each other.** They did not.
  //
  // This filter used to ask `isBuiltinExtension`, whose set is the SEVEN entries
  // of `bundled-extensions.json` with `type: "builtin"`. The composer's
  // extension menu asks `isCapabilityExtension`, whose set is the TWELVE entries
  // of the capability catalog. The five in the gap — code_execution,
  // extensionmanager, skills, todo, chatrecall — ship with Biorouter, are hidden
  // by the chip, and were counted here as if the user had installed them.
  //
  // Worse, a *failed* shipped capability was deliberately kept in the ratio
  // ("rare but still worth surfacing"), which is right about the surfacing and
  // wrong about the ratio: it makes the denominator exactly one larger than the
  // list the user can see, which is how "2 of 3 extensions loaded" appeared over
  // a menu showing two. The failure does still need saying — so it is said, by
  // name, outside the count.
  //
  // One predicate now, from the catalog the chip uses, so the two cannot drift
  // apart again.
  const shipsWithBiorouter = (name: string) => isCapabilityExtension({ name });
  const results = allResults.filter((r) => !shipsWithBiorouter(r.name));
  const shippedFailures = allResults.filter((r) => shipsWithBiorouter(r.name) && !r.success);

  // Rule 3. Record FIRST, unconditionally, and BEFORE any early return: the
  // standing surface must learn of a failure whether or not the transient one
  // is about to speak, and — the half that is easy to miss — a clean load has
  // to CLEAR a record that is no longer true, including the all-shipped load
  // the next line returns on. What comes back is only what the user has not
  // already been shown.
  const unannounced = recordExtensionLoadResults(
    allResults.map((r) => ({
      name: r.name,
      success: r.success,
      error: r.error,
      builtin: shipsWithBiorouter(r.name),
    }))
  );

  if (results.length === 0 && shippedFailures.length === 0) {
    return;
  }

  const failedExtensions = results.filter((r) => !r.success);
  // A shipped capability that failed to load is still a failure for both rules
  // below: it must not be silenced as a background success, and it must not be
  // overwritten by a later clean run. It is only excluded from the *ratio*.
  const anyFailure = failedExtensions.length > 0 || shippedFailures.length > 0;

  if (anyFailure && unannounced.length === 0) {
    // Every failure here has already had its one announcement. Repeating it is
    // the recurrence this rule exists to stop; the Extensions page still shows
    // it, with the error text and something to do about it.
    return;
  }

  if (!anyFailure) {
    // Rule 1: a background chat's clean load is silent.
    const isBackgroundChat =
      sessionId !== undefined &&
      focusedChatSessionId !== null &&
      focusedChatSessionId !== sessionId;
    // Rule 2: never downgrade a live failure toast into a success.
    if (isBackgroundChat || (extensionToastIsFailure && toastService.isExtensionToastActive())) {
      return;
    }
  }

  extensionToastIsFailure = anyFailure;

  // ⚠ `shippedFailures.length === 0` guards the single-error fast path: it
  // renders ONE extension and returns, so taking it while a built-in capability
  // had also failed would drop that failure on the floor entirely.
  if (results.length === 1 && failedExtensions.length === 1 && shippedFailures.length === 0) {
    const failed = failedExtensions[0];
    const errorMsg = failed.error || 'Unknown error';
    const recoverHints = createExtensionRecoverHints(errorMsg);
    const displayMsg = formatExtensionErrorMessage(errorMsg, 'Failed to load extension');

    toastService.error({
      title: failed.name,
      msg: displayMsg,
      traceback: errorMsg,
      recoverHints,
    });
    // Only now — a failure that was never rendered has not been announced, and
    // must keep its one announcement for the load that does render it.
    markExtensionLoadFailuresAnnounced([failed.name]);
    return;
  }

  const extensionStatuses: ExtensionLoadingStatus[] = results.map((r) => {
    const errorMsg = r.error || 'Unknown error';
    return {
      name: r.name,
      status: r.success ? 'success' : 'error',
      error: r.success ? undefined : errorMsg,
      recoverHints: r.success ? undefined : createExtensionRecoverHints(errorMsg),
    };
  });

  toastService.extensionLoading(
    extensionStatuses,
    results.length,
    true,
    shippedFailures.map((r) => r.name)
  );
  markExtensionLoadFailuresAnnounced(unannounced.map((entry) => entry.name));
}
