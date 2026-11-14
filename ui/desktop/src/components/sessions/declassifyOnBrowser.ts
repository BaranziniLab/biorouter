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

/**
 * The line that takes the control's place, on every surface: the refusal and its reason in one
 * line (a refusal is never hover-only, spec 2.6).
 */
export const DECLASSIFY_NEEDS_HOST_SHORT = 'Marking a chat public needs the host';

/**
 * The help behind {@link DECLASSIFY_NEEDS_HOST_SHORT}, for an InfoTip beside it: why the host,
 * and where to go. Two sentences, as an InfoTip allows (owner, 2026-11-13: one line up front, the
 * detail on hover). History's band shows it this way.
 */
export const DECLASSIFY_NEEDS_HOST_HELP =
  "This lowers the chat's privacy, and a browser cannot prove the request is yours. " +
  'On the host running biorouter serve, use the Biorouter app or biorouter session declassify.';

/**
 * The note a row menu shows under its disabled item. A menu row cannot hold an InfoTip (spec
 * 2.6: never inside a menu row, and a menu takes no Tab stop of its own), so the note says the
 * short line, its reason and the way out in two sentences rather than three. It used to be the
 * whole explanation, 330 characters drawn as a block inside the menu.
 */
export const DECLASSIFY_NEEDS_HOST_REASON =
  `${DECLASSIFY_NEEDS_HOST_SHORT}, because a browser cannot prove a request came from you. ` +
  'On the machine running biorouter serve, use the Biorouter app there or run ' +
  '`biorouter session declassify <chat id>`.';

/**
 * Why this surface does not offer declassification, as the help for the short line, or `null`
 * where it does: the desktop, whose daemon holds the key.
 *
 * `null` rather than `''`, for the reason `hostManagedModelReason` gives: a
 * caller that spreads this into a `title` wants the attribute absent on the
 * desktop, not present and empty.
 */
export function declassifyBrowserReason(): string | null {
  return isBrowserSurface() ? DECLASSIFY_NEEDS_HOST_HELP : null;
}
