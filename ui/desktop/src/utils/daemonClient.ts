import type { Config } from '../api/client';
import { coalesceSessionReads } from './sessionReadCoalescing';

/**
 * The renderer's one transport to the daemon.
 *
 * `globalThis.fetch` is resolved per call, as the generated client itself does,
 * so a test that stubs it (and `src/test/networkGuard.ts`) still sees every
 * request. One instance for the renderer: a batch only shares requests issued
 * through the same one.
 */
export const daemonFetch = coalesceSessionReads((request) => globalThis.fetch(request));

/**
 * What `renderer.tsx` hands `client.setConfig` once it knows where the daemon is.
 *
 * ⚠ The secret is the only header here, and the proof of a person is never one of
 * them — `userActionHeaders()` is attached per request, for the reason
 * `utils/userAction.ts` gives. `daemonFetch` attaches nothing; it only lets
 * identical chat reads issued together share one request
 * (`utils/sessionReadCoalescing.ts`).
 */
export function daemonClientConfig(baseUrl: string, secretKey: string): Config {
  return {
    baseUrl,
    headers: {
      'Content-Type': 'application/json',
      'X-Secret-Key': secretKey,
    },
    fetch: daemonFetch,
  };
}
