import { toastError } from '../toasts';
import { localAppLinkId } from './managedAppPreviewPolicy';

/**
 * What the main process answers `open-external` with: nothing for a website (it shows its own
 * confirmation, and a refusal there is the person's own Cancel), and a verdict for a link to an
 * app on this machine, which is opened through a fresh launch link and can fail for reasons the
 * person has to be told (the app is gone, the background service is restarting).
 */
export type OpenExternalResult = { ok: true } | { ok: false; message: string } | void;

/**
 * The failure toast's words, shared by the three surfaces that open a link in the browser (the
 * transcript's artifact card, the preview panel's header and external-page card, a live page's
 * toolbar). They are the words Built apps' Launch uses for the same failure, so an app that will
 * not open reads the same wherever it was clicked. The title is the one line; the main process's
 * own sentence, when it gave one, is the message.
 */
export const OPEN_IN_BROWSER_COPY = {
  failedTitle: 'Couldn’t open the app',
  failed: 'Couldn’t open the app in your browser.',
  bridgeFailed: 'Biorouter couldn’t open that link.',
} as const;

/**
 * Open `url` in the person's browser from a control they clicked, and say why when an app could
 * not be opened. A failure used to end in the main process's log, so "Open in browser" on a live
 * app did nothing at all.
 */
export async function openInBrowser(url: string): Promise<void> {
  let result: unknown;
  try {
    result = await window.electron.openExternal(url);
  } catch {
    result = { ok: false, message: OPEN_IN_BROWSER_COPY.bridgeFailed };
  }
  if (!result || typeof result !== 'object' || (result as { ok?: unknown }).ok !== false) return;
  const message = (result as { message?: unknown }).message;
  toastError({
    title: OPEN_IN_BROWSER_COPY.failedTitle,
    msg: typeof message === 'string' && message.trim() ? message : OPEN_IN_BROWSER_COPY.failed,
  });
}

/**
 * What a live preview's "Open in browser" opens: the address the view was opened at (`url`, from
 * the transcript), or the page's current one (`current`, from the view's state).
 *
 * ⚠ The current address is the PAGE's to set. `history.replaceState` changes it without a
 * navigation, so nothing guards it, and a load that fails still commits the address it failed on.
 * A link to an app on this machine is opened with a fresh launch link minted with this window's
 * secret, so the page must not get to choose which app that is: an app previewed here (whose code
 * is untrusted) could put another app's address in the toolbar, and the person's click would give
 * the system browser an access cookie for THAT app, on the one origin every app shares there. So:
 *
 * - A managed preview (`managedApp`) shows one app and can navigate nowhere else, so it opens that
 *   app: the one `url` names, which is the one the main process scoped it to.
 * - Any other preview opens where it is now, unless that is an app on this machine it was not
 *   opened at; then it opens where it was opened.
 */
export function previewOpenTarget(
  url: string,
  current: { url: string; managedApp?: boolean } | null | undefined
): string {
  if (!current?.url || current.managedApp) return url;
  if (current.url !== url && localAppLinkId(current.url)) return url;
  return current.url;
}
