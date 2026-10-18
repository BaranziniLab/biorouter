/**
 * The production transport for `downloadWithResume`: Electron's `net`, in the
 * same session partition electron-updater uses, so the system proxy, PAC file
 * and certificate store apply exactly as they did before.
 *
 * Redirects are followed by hand, as electron-updater's own executor does:
 * GitHub answers a release download with a 302 to a signed CDN URL on another
 * origin, and credentials must not ride a cross-origin hop. Every resume asks
 * the ORIGINAL URL again, so an expired signed URL is never re-used.
 *
 * Not unit tested: it is a thin adapter over `net`, and only a real Electron
 * process has one. `scripts/run-live-update-e2e.js` drives it through the real
 * electron-updater engine against a server that drops the connection mid-file.
 */
import { net, session } from 'electron';
import type { IncomingMessage } from 'electron';
import type { Readable } from 'stream';
import { DownloadAbortedError, type OpenPart, type PartResponse } from './resumableDownload';

/** electron-updater's partition (`NET_SESSION_NAME`), so nothing about the network changes. */
export const UPDATER_SESSION_PARTITION = 'electron-updater';
const MAX_REDIRECTS = 10;
/** No byte for this long counts as a dropped connection, and the download resumes. */
const IDLE_TIMEOUT_MS = 60_000;
const CREDENTIAL_HEADERS = new Set(['authorization', 'cookie', 'proxy-authorization']);

function withoutCredentials(headers: Record<string, string>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(headers).filter(([name]) => !CREDENTIAL_HEADERS.has(name.toLowerCase()))
  );
}

export function electronNetOpenPart(
  partition = UPDATER_SESSION_PARTITION,
  idleTimeoutMs = IDLE_TIMEOUT_MS
): OpenPart {
  let cachedSession: Electron.Session | null = null;
  const updaterSession = () =>
    (cachedSession ??= session.fromPartition(partition, { cache: false }));

  const open = (
    url: string,
    headers: Record<string, string>,
    signal: AbortSignal,
    redirects: number
  ): Promise<PartResponse> =>
    new Promise<PartResponse>((resolve, reject) => {
      const request = net.request({
        url,
        method: 'GET',
        session: updaterSession(),
        redirect: 'manual',
      });
      for (const [name, value] of Object.entries(headers)) request.setHeader(name, value);

      let response: (IncomingMessage & Readable) | null = null;
      let settled = false;
      let idleTimer: ReturnType<typeof setTimeout> | null = null;

      const clearIdle = () => {
        if (idleTimer) clearTimeout(idleTimer);
        idleTimer = null;
      };
      const detach = () => {
        clearIdle();
        signal.removeEventListener('abort', onAbort);
      };
      // Before the response, a failure rejects the open; after it, it ends the
      // body with that error so the reader's `for await` throws it.
      const fail = (error: Error) => {
        detach();
        if (!settled) {
          settled = true;
          reject(error);
        } else {
          response?.destroy(error);
        }
      };
      const armIdle = () => {
        clearIdle();
        idleTimer = setTimeout(() => {
          request.abort();
          fail(new Error(`net::ERR_TIMED_OUT (no data for ${Math.round(idleTimeoutMs / 1000)} s)`));
        }, idleTimeoutMs);
      };
      const onAbort = () => {
        request.abort();
        fail(new DownloadAbortedError());
      };
      if (signal.aborted) {
        onAbort();
        return;
      }
      signal.addEventListener('abort', onAbort, { once: true });

      request.on('redirect', (_status, _method, redirectUrl) => {
        request.abort();
        detach();
        settled = true;
        if (redirects >= MAX_REDIRECTS) {
          reject(new Error(`Too many redirects (> ${MAX_REDIRECTS})`));
          return;
        }
        const next = new URL(redirectUrl, url);
        const sameOrigin = next.origin === new URL(url).origin;
        open(
          next.href,
          sameOrigin ? headers : withoutCredentials(headers),
          signal,
          redirects + 1
        ).then(resolve, reject);
      });

      request.on('response', (incoming) => {
        const body = incoming as IncomingMessage & Readable;
        response = body;
        armIdle();
        body.on('end', detach);
        body.on('aborted', () =>
          fail(new Error('net::ERR_CONNECTION_CLOSED (the response was aborted)'))
        );
        settled = true;
        resolve({
          status: body.statusCode,
          header: (name) => {
            const value = body.headers[name.toLowerCase()];
            return Array.isArray(value) ? value[0] : value;
          },
          // ⚠ Not a 'data' listener: that switches the stream to flowing mode,
          // and every chunk that arrives before the reader's `for await` starts
          // would be emitted to nobody and lost. Re-arming here keeps the
          // stream paused until it is read.
          body: (async function* () {
            for await (const chunk of body) {
              armIdle();
              yield chunk as Uint8Array;
            }
          })(),
        });
      });

      request.on('error', (error) => fail(error));
      armIdle();
      request.end();
    });

  return (url, headers, signal) => open(url, headers, signal, 0);
}
