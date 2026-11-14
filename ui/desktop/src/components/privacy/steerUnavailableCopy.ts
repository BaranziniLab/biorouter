import { isBrowserSurface } from '../../utils/surface';
import { HOST_SERVE_COMMAND } from './hostManagedModelCopy';

/**
 * What a browser-served session is told INSTEAD of a silent 403 on "Add now".
 *
 * `docs/deployment/serve-decisions.md` **SD-8**: a control a `biorouter serve`
 * session can never use must say so where it sits, rather than take the click
 * and do nothing. The soft interrupt (BR-61 "steer") is exactly such a control,
 * and — unlike Stop, which SD-11 handed to the reach gate — it still asks for
 * the user-action proof on every daemon.
 *
 * ⚠ **A browser surface can never steer, and that is provable rather than
 * likely.** Two facts meet:
 *
 * - `utils/userAction.ts` → `userActionHeaders()` emits `X-User-Action` on the
 *   desktop **only**. On a browser surface it returns `X-Caller-Provider` and
 *   nothing else, because there is no Electron main process to mint a key from.
 * - `crates/biorouter-server/src/routes/reply.rs` → `steer_refusal()` admits
 *   `UserActionProof::Proven` and nothing else. Without that header a daemon
 *   holding a key answers `Unproven` (an empty 403) and one holding none
 *   answers `NoKeyInstalled` (403 carrying `STEER_NO_KEY`).
 *
 * Both kinds of daemon therefore refuse, for the whole life of the page — which
 * is what licenses explaining this BEFORE the click rather than after it.
 *
 * ⚠ **These strings are for a human**, on the rule `hostManagedModelCopy.ts`
 * records for its own: the daemon's `STEER_NO_KEY` is addressed to an AI agent —
 * it opens by naming a key the reader never chose, and its "nothing was queued"
 * clause is an answer to a request this surface no longer makes. Nothing here is
 * copied back into that constant, which is privacy-critical and pinned by Rust
 * tests.
 *
 * What IS kept from the daemon's refusal is its useful half — *"Stop the turn
 * and send the message instead"* — because here that is not advice about
 * somewhere else: **Stop & send** is the button in the same queue row, one
 * control to the right of the one that is missing.
 */

/** Heading for the control that is absent because this surface cannot prove a person. */
export const STEER_UNAVAILABLE_TITLE = 'A running turn cannot be steered from a browser';

/**
 * The one line the queue shows, wherever it says this (owner, 2026-11-13: one line up front, the
 * detail on hover). Why is {@link STEER_UNAVAILABLE_WHY}, behind the info icon beside it.
 */
export const STEER_UNAVAILABLE_SHORT =
  'A message cannot join a running turn in a browser. Use Stop & send instead.';

/** The info icon's text beside {@link STEER_UNAVAILABLE_SHORT}: why, and where steering works. */
export const STEER_UNAVAILABLE_WHY =
  `The machine running ${HOST_SERVE_COMMAND} cannot tell a request from you apart from one made ` +
  `by the model, and steering a running turn needs that proof. Steering works in the desktop app.`;

/** The info icon's name: "About steering in a browser". */
export const STEER_UNAVAILABLE_NAME = 'steering in a browser';

/**
 * The full explanation, as one string: the reason {@link steerUnavailableReason} returns. The
 * queue draws it as the line and its info icon; this joins the two halves for a caller that needs
 * the whole reason as text.
 */
export const STEER_UNAVAILABLE_REASON = `${STEER_UNAVAILABLE_SHORT} ${STEER_UNAVAILABLE_WHY}`;

/**
 * The reason a steer is unavailable on this surface, or `null` where it works.
 *
 * `null` rather than `''` for the reason `hostManagedModelReason` gives: a
 * caller spreading it into a `title` wants the attribute absent, not empty.
 *
 * Asked fresh on every call and never captured at import time — `isBrowserSurface`
 * reads the DOM marker `renderer.tsx` stamps, and a module-level snapshot would
 * be taken before that marker exists.
 */
export function steerUnavailableReason(): string | null {
  return isBrowserSurface() ? STEER_UNAVAILABLE_REASON : null;
}
