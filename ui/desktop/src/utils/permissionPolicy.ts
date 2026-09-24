import path from 'node:path';
import { fileURLToPath } from 'node:url';

export function isAppOrigin(candidate: string, appUrl: URL): boolean {
  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    return false;
  }

  if (appUrl.protocol !== 'file:') return url.origin === appUrl.origin;
  if (url.protocol !== 'file:') return false;

  try {
    const entry = fileURLToPath(appUrl);
    const rendererDir = path.dirname(entry);
    const target = path.resolve(fileURLToPath(url));
    return target === entry || target.startsWith(rendererDir + path.sep);
  } catch {
    return false;
  }
}

export function shouldOpenExternalNavigation(candidate: string, appUrl: URL): boolean {
  if (candidate.length > 8 * 1024) return false;
  if (isAppOrigin(candidate, appUrl)) return false;
  try {
    const url = new URL(candidate);
    return (
      (url.protocol === 'http:' || url.protocol === 'https:') &&
      url.username === '' &&
      url.password === ''
    );
  } catch {
    return false;
  }
}

/**
 * Where an artifact preview frame is allowed to navigate: nowhere.
 *
 * Every surface that displays a generated artifact — the side panel, the
 * "open in browser" wrapper document — hands the figure to the frame as a
 * `srcdoc`, so the only legitimate destinations are `about:srcdoc` and the
 * `about:blank` the frame starts at. Anything else is the guest document trying
 * to move the frame somewhere, which is exactly what must not happen.
 *
 * This used to carry a second allowance: the daemon's `/mcp-ui-proxy`, which
 * served the figure to an inline iframe in the transcript. That surface is gone
 * — a figure is only ever displayed in the artifact panel now — and with it the
 * one reason this function needed to know the daemon's origin at all. Losing the
 * parameter is a tightening, not a regression: the policy is now a closed set of
 * two literals with nothing configurable about it.
 */
export function isAllowedArtifactFrameNavigation(candidate: string): boolean {
  return candidate === 'about:srcdoc' || candidate === 'about:blank';
}

/**
 * The permissions Biorouter's own renderer may hold. Everything not named here
 * is denied, including every permission Electron adds in a later release.
 *
 * `main.ts`'s `installSessionHooks` routes BOTH of the renderer partition's
 * handlers through this one function: the check handler (what
 * `navigator.permissions.query` and Chromium's own pre-checks see) and the
 * request handler, so a grant here is the whole grant and a denial here is the
 * whole denial. Which handler hears what, measured on Electron 39.8.10:
 * `navigator.permissions.query({ name: 'clipboard-write' })` calls the check
 * handler (twice) and never the request handler, and `writeText` calls the
 * request handler once and never the check handler.
 *
 * Two grants, each only to a document that `isAppOrigin` recognises as the
 * renderer itself:
 *
 * - **`clipboard-sanitized-write`**, the permission behind
 *   `navigator.clipboard.writeText` and `write`. Every Copy control in the
 *   renderer uses one of the two (chat, Crew's copy fields, the timeline, the
 *   sidebar announcer). When these handlers moved onto the renderer's
 *   partition, this permission fell under the audio-only rule below and every
 *   Copy in the app failed with `NotAllowedError`. Chromium sanitises what the
 *   write may put on the pasteboard, and a write reads nothing back.
 *
 *   ⚠ This grant only covers a write that carries user activation (a real
 *   click on Copy). A write WITHOUT activation, such as a bare DevTools or CDP
 *   `writeText`, never asks for `clipboard-sanitized-write`. Chromium asks for
 *   its unsanitised read-write permission instead, which Electron calls
 *   `clipboard-read`, and THIS policy's denial of `clipboard-read` is what
 *   refuses it. Measured on Electron 39.8.10 with a `persist:` partition wired
 *   as `main.ts` wires it: no check-handler call, one request-handler call for
 *   `clipboard-read` carrying the entry's full URL, denied, then
 *   `NotAllowedError: Write permission denied`. With `clipboard-read` granted
 *   to the app, the same bare write succeeded. So granting `clipboard-read`
 *   (for a paste feature, say) also lets the renderer write the clipboard with
 *   no click at all. Decide those two together.
 * - **`media`, audio only**, for dictation.
 *
 * `clipboard-read` (and `deprecated-sync-clipboard-read`) stay DENIED. A read
 * sees whatever the user last copied anywhere on the machine, and nothing in
 * the renderer reads the clipboard. The same denial is what refuses a write
 * made without a click (see above).
 *
 * What `requestingUrl` is, measured on Electron 39.8.10 with a `persist:`
 * partition: for `clipboard-sanitized-write`, the check handler receives
 * `requestingOrigin` = `file:///` for every packaged document (all `file:`
 * pages share that one origin) and `details.requestingUrl` = the document's full
 * committed URL, hash included; the request handler receives the same full URL.
 * `main.ts` passes `details.requestingUrl || requestingOrigin`, so this sees the
 * full URL. ⚠ Do not "fix" a bare `file:///` into a grant: it names no path, so
 * accepting it would grant every local file the renderer partition ever
 * displays. It fails `isAppOrigin` (it resolves to `/`), which is the intended
 * answer.
 */
export function isAllowedRendererPermission(
  permission: string,
  requestingUrl: string,
  appUrl: URL,
  mediaTypes: ReadonlyArray<string>
): boolean {
  if (permission === 'clipboard-sanitized-write') return isAppOrigin(requestingUrl, appUrl);
  return (
    permission === 'media' &&
    mediaTypes.length > 0 &&
    mediaTypes.every((type) => type === 'audio') &&
    isAppOrigin(requestingUrl, appUrl)
  );
}

/**
 * Whether the embedded browser may navigate to this URL at all.
 *
 * Lives here rather than in `embeddedBrowser.ts` for the same reason as its
 * siblings: this file imports nothing from Electron, so the policy is testable
 * on its own. `file:` and `data:` are refused outright, and so is any URL
 * carrying credentials — a visible host that is not the host contacted is a
 * phishing primitive, not a convenience.
 */
export function isNavigableEmbeddedUrl(candidate: string): boolean {
  if (candidate.length > 8 * 1024) return false;
  try {
    const url = new URL(candidate);
    return (
      (url.protocol === 'http:' || url.protocol === 'https:') &&
      url.username === '' &&
      url.password === ''
    );
  } catch {
    return false;
  }
}
