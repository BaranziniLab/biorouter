import { isBrowserSurface } from '../../../utils/surface';

/**
 * What a browser-served page says in place of the Reset controls.
 *
 * `docs/deployment/serve-decisions.md` **SD-8**: a control that can never work
 * on this surface declares itself unavailable *before* the user acts on it —
 * the model picker (`hostManagedModelCopy.ts`), a delegated subagent's tab
 * (`subagentReadOnly.ts`) and "Make this chat public" (`declassifyOnBrowser.ts`)
 * are the existing cases, and this is the next.
 *
 * ⚠ **It cannot be made to work here, and that is the ruling.** `POST /reset`
 * deletes every chat, knowledge base, schedule, skill, extension, workflow or
 * built app in the areas it names, private ones included, so it answers only a
 * request carrying the `X-User-Action` proof. A `serve` daemon is started with
 * stdin closed and holds no digest to check that proof against (SD-7), so it
 * refuses every reset, for everyone. It used to refuse none: the route took no
 * headers, and any caller holding the daemon secret — which a public chat's
 * shell can read — could empty History.
 *
 * ⚠ **These strings are for the person looking at the panel.** The daemon's own
 * refusals (`RESET_NEEDS_USER`, `RESET_NO_USER_KEY` in `routes/reset.rs`) are
 * written for whatever reads an error body; nothing here should be copied back
 * into them.
 *
 * ⚠ **It names the HOST, not "the desktop app"**, for the reason
 * `declassifyOnBrowser.ts` gives: the data lives where the daemon runs, so that
 * is where the control works.
 */
export const RESET_NEEDS_HOST_REASON =
  'This page is served to a browser, which has no way to prove a request came from you rather ' +
  'than from a model. Resetting deletes data for good, so it needs that proof. On the machine ' +
  'running biorouter serve, reset from Settings in the Biorouter app there, or remove items one ' +
  'at a time from a terminal there with `biorouter session remove` and its siblings.';

/**
 * Why this surface does not offer a reset, or `null` where it does — the
 * desktop, whose daemon holds the key.
 */
export function resetBrowserReason(): string | null {
  return isBrowserSurface() ? RESET_NEEDS_HOST_REASON : null;
}
