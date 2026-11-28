import https from 'node:https';
import { isIP, type LookupFunction } from 'node:net';
import { Readable } from 'node:stream';
import { resolvePublicEmbeddedHost, type PublicNetworkTarget } from './embeddedBrowserPolicy';
import { remoteImageTarget, type RemoteImageResult } from './remoteImage';

/**
 * The main process's half of click-to-load remote images (OP-P0-09; the
 * renderer half and the reasons are in `remoteImage.ts`).
 *
 * A person clicked one image in a chat. This fetches exactly that address and
 * hands its bytes back, so the renderer shows it from a `blob:` URL and its own
 * page policy can refuse every web image (`img-src 'self' data: blob:` in
 * `main.ts`). The request carries nothing that ties it to anything else the app
 * has loaded:
 *
 * - **No cookie, no referrer, no cache.** Node's `https` module keeps no cookie
 *   jar and no HTTP cache, and sends only the headers written below. A clicked
 *   image drawn by the renderer would have stored the host's cookie in the
 *   persistent renderer partition and sent it on every later click, in any chat:
 *   a lasting identifier for whoever placed the image.
 * - **Never a private address.** Every hop is resolved and refused when it
 *   points at a loopback, private, link-local or otherwise non-public address,
 *   the rule the Biorouter Browser and `fetch-metadata` apply, so a public
 *   address cannot redirect a click into the lab's network. The connection then
 *   goes to the address that was checked (`pinnedRequestOptions`), never to a
 *   second lookup of the name, so a host whose DNS answer changes between the
 *   check and the connection (DNS rebinding) cannot reach a private address
 *   either. The certificate is still checked against the name.
 * - **Not through the app's proxy.** This request does not use the proxy the
 *   renderer session is given (`configureProxy` in `main.ts`), so on a network
 *   that allows the web only through a proxy the click shows the unavailable
 *   chip. That fails closed.
 * - **Bounded.** `https:` only (an `http:` hop is upgraded), at most
 *   {@link REMOTE_IMAGE_MAX_REDIRECTS} redirects, {@link REMOTE_IMAGE_MAX_BYTES}
 *   of body and {@link REMOTE_IMAGE_TIMEOUT_MS} in all, and the answer must say it
 *   is an image.
 */

/** Largest image a click will load. Larger than any figure a chat shows. */
export const REMOTE_IMAGE_MAX_BYTES = 20 * 1024 * 1024;

/** How long one click may take, redirects included. */
export const REMOTE_IMAGE_TIMEOUT_MS = 20_000;

/** Redirects followed before giving up. */
export const REMOTE_IMAGE_MAX_REDIRECTS = 5;

/** The response types a clicked image may have: `image/` and a plain subtype. */
const IMAGE_MIME = /^image\/[a-z0-9][a-z0-9.+-]*$/;

/** Requests `url`, connecting to `pinned` and to no other address. */
type RemoteImageFetch = (
  url: string,
  init: RequestInit,
  pinned: PublicNetworkTarget
) => Promise<Response>;

export interface RemoteImageFetchOptions {
  /** The request function. By default `pinnedHttpsFetch`, which keeps no cookies or cache. */
  fetch: RemoteImageFetch;
  /** Resolves a host to the public address to connect to; throws when any of its addresses is not public. */
  resolveHost: (hostname: string) => Promise<PublicNetworkTarget>;
  timeoutMs: number;
  maxBytes: number;
}

/**
 * The options for one `https.request` that connects to `pinned` whatever name
 * the socket layer asks about. The URL's own name is still the TLS server name
 * and the `Host` header, so the certificate is checked against it. No agent, so
 * no connection is kept for a later click.
 */
export function pinnedRequestOptions(
  url: string,
  init: RequestInit,
  pinned: PublicNetworkTarget
): https.RequestOptions {
  const target = new URL(url);
  if (target.protocol !== 'https:') throw new Error('A clicked image is fetched over https only.');
  const hostname = target.hostname.replace(/^\[(.*)\]$/, '$1');
  const lookup = ((_name, options, callback) => {
    if (options?.all) callback(null, [{ address: pinned.address, family: pinned.family }]);
    else callback(null, pinned.address, pinned.family);
  }) as LookupFunction;
  return {
    protocol: 'https:',
    hostname,
    servername: isIP(hostname) ? undefined : hostname,
    port: target.port ? Number(target.port) : 443,
    path: `${target.pathname}${target.search}`,
    method: init.method ?? 'GET',
    headers: Object.fromEntries(new Headers(init.headers)),
    lookup,
    agent: false,
    rejectUnauthorized: true,
  };
}

/** Statuses whose response carries no body, which a `Response` refuses one for. */
const NULL_BODY_STATUSES = new Set([101, 103, 204, 205, 304]);

/** `https.request` to the pinned address, answered as a `Response`. */
function pinnedHttpsFetch(
  url: string,
  init: RequestInit,
  pinned: PublicNetworkTarget
): Promise<Response> {
  return new Promise((resolve, reject) => {
    const request = https.request(
      { ...pinnedRequestOptions(url, init, pinned), signal: init.signal ?? undefined },
      (incoming) => {
        const status = incoming.statusCode ?? 0;
        const headers = new Headers();
        for (const [name, value] of Object.entries(incoming.headers)) {
          if (value === undefined || name === 'set-cookie') continue;
          for (const item of Array.isArray(value) ? value : [value]) headers.append(name, item);
        }
        const nullBody = NULL_BODY_STATUSES.has(status);
        try {
          resolve(
            new Response(
              nullBody ? null : (Readable.toWeb(incoming) as ReadableStream<Uint8Array>),
              { status, headers }
            )
          );
          if (nullBody) incoming.resume();
        } catch (error) {
          // A status a `Response` cannot carry (under 200 or over 599).
          incoming.destroy();
          reject(error);
        }
      }
    );
    request.on('error', reject);
    request.end();
  });
}

const DEFAULT_OPTIONS: RemoteImageFetchOptions = {
  fetch: pinnedHttpsFetch,
  resolveHost: (hostname) => resolvePublicEmbeddedHost(hostname),
  timeoutMs: REMOTE_IMAGE_TIMEOUT_MS,
  maxBytes: REMOTE_IMAGE_MAX_BYTES,
};

function mimeEssence(contentType: string | null): string | null {
  const essence = contentType?.split(';', 1)[0]?.trim().toLowerCase() ?? '';
  return IMAGE_MIME.test(essence) ? essence : null;
}

/** The body, or null as soon as it grows past `maxBytes`. */
async function readCapped(response: Response, maxBytes: number): Promise<Uint8Array | null> {
  if (!response.body) return new Uint8Array(0);
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > maxBytes) {
      await reader.cancel().catch(() => undefined);
      return null;
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

async function discard(response: Response): Promise<void> {
  await response.body?.cancel().catch(() => undefined);
}

/** Fetch the one image a person clicked. Never throws. */
export async function fetchRemoteImage(
  raw: unknown,
  overrides: Partial<RemoteImageFetchOptions> = {}
): Promise<RemoteImageResult> {
  const options = { ...DEFAULT_OPTIONS, ...overrides };
  const first = remoteImageTarget(raw);
  if (!first) return { ok: false, reason: 'invalid' };

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), options.timeoutMs);
  try {
    let target = first.url;
    for (let hop = 0; hop <= REMOTE_IMAGE_MAX_REDIRECTS; hop++) {
      let pinned: PublicNetworkTarget;
      try {
        pinned = await options.resolveHost(new URL(target).hostname);
      } catch {
        return { ok: false, reason: 'not-public' };
      }
      const response = await options.fetch(
        target,
        {
          method: 'GET',
          redirect: 'manual',
          credentials: 'omit',
          referrerPolicy: 'no-referrer',
          cache: 'no-store',
          headers: { Accept: 'image/avif,image/webp,image/apng,image/svg+xml,image/*;q=0.8' },
          signal: controller.signal,
        },
        pinned
      );

      const location = response.headers.get('location');
      if (response.status >= 300 && response.status < 400 && location) {
        await discard(response);
        let next: URL;
        try {
          next = new URL(location, target);
        } catch {
          return { ok: false, reason: 'unavailable' };
        }
        const hopTarget = remoteImageTarget(next.href);
        if (!hopTarget) return { ok: false, reason: 'invalid' };
        target = hopTarget.url;
        continue;
      }

      if (!response.ok) {
        await discard(response);
        return { ok: false, reason: 'unavailable' };
      }
      const mimeType = mimeEssence(response.headers.get('content-type'));
      if (!mimeType) {
        await discard(response);
        return { ok: false, reason: 'not-an-image' };
      }
      const declared = Number(response.headers.get('content-length'));
      if (Number.isFinite(declared) && declared > options.maxBytes) {
        await discard(response);
        return { ok: false, reason: 'too-large' };
      }
      const bytes = await readCapped(response, options.maxBytes);
      if (!bytes) return { ok: false, reason: 'too-large' };
      return { ok: true, mimeType, bytes };
    }
    return { ok: false, reason: 'unavailable' };
  } catch {
    return { ok: false, reason: 'unavailable' };
  } finally {
    clearTimeout(timer);
  }
}
