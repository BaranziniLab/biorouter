/**
 * Remote images in markdown load on a person's click (OP-P0-09, the owner's
 * answer to D15).
 *
 * An `<img>` with a web address sends a request the moment it is drawn: a read
 * receipt, the reader's network address, and whatever the address itself
 * carries. A model steered by a web page or a connector can put a chat's text
 * in that address, so a remote image in a message, a tool result or a previewed
 * file is a button naming its host until the person clicks it. The click loads
 * that one image, and nothing remembers it: the next time the message is drawn,
 * in this chat or any other, it is a button again.
 *
 * This module is the part both processes share, so it imports nothing from
 * either. The renderer half is `RemoteMarkdownImage` in
 * `components/MarkdownContent.tsx`; the main-process half, which fetches the
 * clicked image with no cookie, referrer or cache, is `remoteImageFetch.ts`.
 */

/** The IPC channel the renderer asks the main process to fetch a clicked image on. */
export const REMOTE_IMAGE_CHANNEL = 'load-remote-image';

/** Longest address offered as a remote image, the limit external links already use. */
export const REMOTE_IMAGE_MAX_URL_LENGTH = 8 * 1024;

/** Why a clicked image could not be shown. Never a sentence, so the renderer owns the wording. */
export type RemoteImageFailure =
  | 'invalid'
  | 'not-public'
  | 'unavailable'
  | 'too-large'
  | 'not-an-image';

/** What the main process answers for one clicked image. */
export type RemoteImageResult =
  | { ok: true; mimeType: string; bytes: Uint8Array }
  | { ok: false; reason: RemoteImageFailure };

/** A remote image a person may load: the address that will be requested and its host. */
export interface RemoteImageTarget {
  /** The `https:` address that will be requested (an `http:` one is upgraded). */
  url: string;
  /** The host to name to the person, in ASCII, with a port when there is one. */
  host: string;
}

/**
 * The address a remote markdown image would be requested from, or null when it
 * is not one a person may load.
 *
 * Only an address that names the `http:` or `https:` scheme qualifies, read the
 * way the URL parser reads it (so `https:/host` and `https:\\host` name `host`).
 * `http:` is asked for over `https:`, the upgrade the renderer's page policy
 * always made (`upgrade-insecure-requests`), so a click never sends a chat's
 * address in clear text. An address carrying a user name or password is refused:
 * it would send a credential, and its host is the part that can be faked in
 * front of it. The host is the URL parser's own, so an internationalised name is
 * shown in its ASCII form and a look-alike cannot pass for the real one.
 */
export function remoteImageTarget(raw: unknown): RemoteImageTarget | null {
  if (typeof raw !== 'string') return null;
  const value = raw.trim();
  if (!value || value.length > REMOTE_IMAGE_MAX_URL_LENGTH) return null;
  // A scheme-relative `//host/x.png` names no scheme, so it is never offered
  // here. It is not a file either: on Windows `//host` and `\\host` are network
  // paths, so `resolveMarkdownImageSource` refuses them (an unavailable chip),
  // and the main process's file read refuses them before touching the disk.
  if (!/^https?:/i.test(value)) return null;
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
  if (url.username || url.password || !url.hostname) return null;
  if (url.protocol === 'http:') {
    url.protocol = 'https:';
  }
  return { url: url.href, host: url.host };
}
