import type { BrowserWindow } from 'electron';
import type { BiorouterdResult } from '../biorouterd';
import type { ManagedAppPreviewBackend } from './managedAppPreviewPolicy';

const revokedBackends = new WeakSet<BiorouterdResult['process']>();

/**
 * Main-process provenance; a renderer-provided URL cannot mint this grant.
 *
 * `appOrigin` is where the daemon serves apps to a browser, when that is not
 * `result.baseUrl` (a shared daemon, reached through the desktop's proxy; see
 * {@link fetchDaemonAppOrigin}). `lifetime` revokes the backend early: the main
 * process aborts it when the shared daemon it names is lost or replaced, which
 * `result.process` never reports, because a shared daemon outlives the app.
 */
export function bindManagedAppPreviewBackend(
  result: BiorouterdResult,
  owner: BrowserWindow,
  secretKey?: string,
  appOrigin?: string,
  lifetime?: AbortSignal
): ManagedAppPreviewBackend | undefined {
  if (!result.managed) return undefined;
  const controller = new AbortController();
  const revoke = () => {
    controller.abort();
    result.process.removeListener('exit', revokeBackend);
    result.process.removeListener('error', revokeBackend);
    owner.removeListener('closed', revoke);
    lifetime?.removeEventListener('abort', revoke);
  };
  const revokeBackend = () => {
    revokedBackends.add(result.process);
    revoke();
  };
  // close can lag exit indefinitely when descendants inherit stdio.
  result.process.once('exit', revokeBackend);
  result.process.once('error', revokeBackend);
  owner.once('closed', revoke);
  lifetime?.addEventListener('abort', revoke, { once: true });
  if (
    result.process.exitCode !== null ||
    result.process.signalCode != null ||
    revokedBackends.has(result.process) ||
    result.process.killed ||
    owner.isDestroyed() ||
    lifetime?.aborted
  ) {
    revoke();
  }
  return {
    baseUrl: result.baseUrl,
    ...(appOrigin ? { appOrigin } : {}),
    signal: controller.signal,
    secretKey,
  };
}

/** How long to wait for the daemon to say where it serves apps. */
const APP_ORIGIN_TIMEOUT_MS = 10_000;

/**
 * `candidate` as an origin the managed preview may be sent to:
 * `http://127.0.0.1:<port>` and nothing else. A daemon listening on every
 * interface reports `0.0.0.0`, and `localhost` names the same listener; both
 * become `127.0.0.1`, which is the only host the preview's transport connects
 * to. `undefined` for anything else (another host, IPv6, https, a path).
 */
export function loopbackAppOrigin(candidate: unknown): string | undefined {
  if (typeof candidate !== 'string') return undefined;
  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    return undefined;
  }
  if (
    url.protocol !== 'http:' ||
    !['127.0.0.1', '0.0.0.0', 'localhost'].includes(url.hostname) ||
    !url.port ||
    url.username ||
    url.password ||
    url.pathname !== '/' ||
    url.search ||
    url.hash
  ) {
    return undefined;
  }
  return `http://127.0.0.1:${url.port}`;
}

/**
 * Where the daemon at `baseUrl` serves apps to a browser: its own listener, as
 * its `GET /headless/health` names it (`api_base_url`), asked with the secret.
 *
 * The desktop reaches a shared daemon through a local proxy that admits only
 * requests carrying the desktop's secret, so the proxy's address is no use to a
 * page; and the runtime record that names the daemon holds only its private
 * socket. The daemon itself knows where it listens, and every daemon this
 * desktop attaches to answers this route. `undefined` when it does not answer,
 * or names somewhere a preview may not go ({@link loopbackAppOrigin}).
 */
export async function fetchDaemonAppOrigin(
  baseUrl: string,
  secretKey: string,
  fetchImpl: typeof fetch = fetch
): Promise<string | undefined> {
  try {
    const response = await fetchImpl(`${baseUrl.replace(/\/+$/, '')}/headless/health`, {
      headers: { 'X-Secret-Key': secretKey },
      redirect: 'error',
      signal: AbortSignal.timeout(APP_ORIGIN_TIMEOUT_MS),
    });
    if (!response.ok) return undefined;
    const body = (await response.json().catch(() => null)) as { api_base_url?: unknown } | null;
    return loopbackAppOrigin(body?.api_base_url);
  } catch {
    return undefined;
  }
}
