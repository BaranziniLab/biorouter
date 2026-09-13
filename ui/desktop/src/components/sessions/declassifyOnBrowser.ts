import { isBrowserSurface } from '../../utils/surface';

/**
 * What a browser-served page says in place of "Make this chat public".
 *
 * `docs/deployment/serve-decisions.md` **SD-8**: a control that can never work
 * on this surface declares itself unavailable *before* the user acts on it. The
 * model picker (`hostManagedModelCopy.ts`) and a delegated subagent's tab
 * (`subagentReadOnly.ts`) are the two existing cases; this is the third, and it
 * is the one where getting it wrong costs the most, because the control it
 * guards is the only one in the product that LOWERS a chat's privacy.
 *
 * ⚠ **Measured on a real `biorouter serve` (2026-09-12).** The row menu offered
 * "Make this chat public" with no `aria-disabled`, no `title` and no note; the
 * destructive-confirm dialog then asked the user to type the last six characters
 * of the chat id; and `POST /sessions/{id}/declassify` answered **403** with the
 * daemon's model-facing sentence, which closes *"stop and ask the user to mark
 * it public from the chat history"*. The reader was the user, in the chat
 * history. Both halves of that are fixed: the daemon now has a sentence for a
 * keyless caller (`DECLASSIFY_NO_USER_KEY` in `routes/session.rs`), and this
 * surface no longer produces the request at all.
 *
 * ⚠ **It cannot be made to work here, and that is the ruling, not a limitation
 * waiting to be lifted.** The `X-User-Action` digest is the only thing that
 * distinguishes a person from a model to the daemon, and a `serve` daemon is
 * started with stdin closed, so it holds none (SD-7). Admitting the browser's
 * `X-Secret-Key` in its place would admit every caller that can read that secret
 * — which issue #56 §9.3 A1 measured to be any developer-enabled agent shell —
 * to the one operation that reverses the privacy ratchet. The second gate cannot
 * cross the gap either: a chat graded onto the typed control also needs the
 * operating system to confirm the user (DR-20), and that prompt is raised on the
 * machine running the daemon, which is not where the browser is.
 *
 * ⚠ **These strings are for the person looking at the row.** The daemon's own
 * refusals are written for whatever reads an error body, and nothing here should
 * be copied back into them.
 *
 * ⚠ **It names the HOST, not "the desktop app".** The desktop application runs a
 * daemon of its own over its own store; a browser pointed at someone else's
 * `serve` host would find nothing there to mark public. The capability lives
 * where the daemon lives, which is also where `biorouter session declassify`
 * runs, so that is what this says.
 */

/** The full reason, for the note that takes the control's place. */
export const DECLASSIFY_NEEDS_HOST_REASON =
  'This page is served to a browser, which has no way to prove a request came from you rather ' +
  'than from a model. Marking a chat public is the one change that lowers its privacy, so it ' +
  'needs that proof. On the machine running biorouter serve, mark it public in the Biorouter ' +
  'app there, or run `biorouter session declassify <chat id>`.';

/** The line for a row that has space for no more, and for a `title`. */
export const DECLASSIFY_NEEDS_HOST_SHORT = 'Marking a chat public needs the host';

/**
 * Why this surface does not offer declassification, or `null` where it does —
 * the desktop, whose daemon holds the key.
 *
 * `null` rather than `''`, for the reason `hostManagedModelReason` gives: a
 * caller that spreads this into a `title` wants the attribute absent on the
 * desktop, not present and empty.
 */
export function declassifyBrowserReason(): string | null {
  return isBrowserSurface() ? DECLASSIFY_NEEDS_HOST_REASON : null;
}
