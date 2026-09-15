import { isBrowserSurface } from '../../utils/surface';

/**
 * What a browser-served page says about capturing the preview panel.
 *
 * Both captures of the panel — a person's "Send a region to the chat" and the
 * agent's `workspace_read_panel { capture: true }` — end in
 * `window.electron.captureRegion`, a compositor grab in Electron's main process.
 * That is the only thing that can see into the sandboxed `srcdoc` frames most
 * previews render in, and a page served by `biorouter serve` has no main process:
 * the bridge `renderer.tsx` installs there carries no `captureRegion` at all.
 *
 * ⚠ **It cannot be made to work here, and the reason is not a policy.** A
 * DOM-walking screenshot library is not a fallback: it returns an empty box for
 * a figure in a sandboxed frame, which would hand the model a blank image
 * captioned as the user's selection. So `docs/deployment/serve-decisions.md`
 * **SD-8** applies: the control says it is unavailable before it is touched
 * (`resetOnBrowser.ts`, `declassifyOnBrowser.ts` and `hostManagedModelCopy.ts`
 * are the earlier cases), and the agent is told a capture can never succeed
 * here, rather than that it failed "right now", which reads as an invitation to
 * retry.
 *
 * Until 2026-09-14 neither was true. Both call sites optional-chained only the
 * bridge, so in a browser the call threw "captureRegion is not a function": the
 * region overlay stayed up after the drag, and the agent was handed that
 * TypeError as the tool's result.
 */

/** For the person, on the disabled camera control in the panel's header. */
export const ANNOTATE_NEEDS_DESKTOP_REASON =
  'Sending a region needs the Biorouter desktop app: a web browser cannot take a picture of ' +
  'this preview.';

/** For the agent, as the panel command's `detail` when a capture comes back empty here. */
export const PANEL_CAPTURE_NEEDS_DESKTOP_DETAIL =
  'the panel cannot be captured here: this chat is open in a web browser (biorouter serve), ' +
  'which has no way to take a screenshot of the preview; read its text instead ' +
  '(workspace_read_panel without capture)';

/**
 * Why this surface does not offer "Send a region to the chat", or `null` where
 * it does — the desktop app.
 */
export function annotateBrowserReason(): string | null {
  return isBrowserSurface() ? ANNOTATE_NEEDS_DESKTOP_REASON : null;
}
