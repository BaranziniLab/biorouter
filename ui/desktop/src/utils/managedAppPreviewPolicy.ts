import { requestAppLaunch } from './appLaunchLink';

export type ManagedAppPreviewBackend = {
  /**
   * Where the main process asks the daemon for an app's launch link, with
   * `secretKey`: the daemon itself, or the desktop's local proxy in front of a
   * shared daemon. Never an address the preview's page is sent to.
   */
  baseUrl: string;
  /**
   * Where the daemon serves apps to a browser: its own listener,
   * `http://127.0.0.1:<port>`. The preview's page, its requests and its socket
   * go here and nowhere else.
   *
   * ⚠ Not `baseUrl` for a shared daemon. The proxy there admits only requests
   * carrying the desktop's secret, which a page never sends, so a preview
   * sent to the proxy got an empty 403 for its page, bundle and agent socket.
   * Absent, it is `baseUrl`: a per-window daemon, which the desktop reaches
   * directly.
   */
  appOrigin?: string;
  signal: AbortSignal;
  /**
   * The daemon secret, held in the main process only. It is sent once per
   * preview, to ask the daemon for the app's launch link
   * (`managedAppLaunchUrl`), and never to the page or on its requests.
   */
  secretKey?: string;
};

export type ManagedAppPreviewScope = {
  backend: ManagedAppPreviewBackend;
  origin: string;
  port: number;
  appId: string;
  rootPath: string;
};

const APP_PAGE = /^\/apps\/([A-Za-z0-9_-]{1,128})\/?$/;

/**
 * How a link to an app on this machine is spelled, before anything parses it:
 * plain http to `127.0.0.1`, on any port, followed directly by a path. Checked
 * on the raw string because the URL parser turns `127.1`, `2130706433` and
 * `0x7f.1` into `127.0.0.1`, and `127.0.0.1:1@evil.test` into a link to
 * `evil.test`.
 *
 * ⚠ Only the spellings the daemon itself writes (`launch_app`, the launch
 * answer, `BIOROUTER_APP_BASE_URL`), which are its bound address with an
 * unspecified one made loopback. Not `localhost`: that is what a web development
 * server prints, so `http://localhost:5173/apps/dashboard/` is far more often
 * the person's own project than a Biorouter app, and rebinding it by path opened
 * whichever Biorouter app happened to share the name.
 */
const LOCAL_APP_LINK = /^http:\/\/127\.0\.0\.1(?::\d{1,5})?\/apps\//;

/** {@link LOCAL_APP_LINK}, or the IPv6 loopback a daemon listening on `[::]` names. */
const LOCAL_LINK_TO_OPEN = /^http:\/\/(?:127\.0\.0\.1|\[::1\])(?::\d{1,5})?\/apps\//;

function safeUrl(candidate: string): URL | null {
  try {
    // Reject ambiguous spellings before URL normalizes dot segments/escapes.
    if (/[\\\s]/.test(candidate)) return null;
    const rawPath = candidate.replace(/^[a-z]+:\/\/[^/]+/i, '').split(/[?#]/, 1)[0];
    if (rawPath.includes('%') || rawPath.split('/').some((part) => part === '.' || part === '..')) {
      return null;
    }
    const url = new URL(candidate);
    return url.username || url.password ? null : url;
  } catch {
    return null;
  }
}

/** The backend's app origin, when it is one a preview may be sent to; else `null`. */
function appOriginOf(backend: ManagedAppPreviewBackend): URL | null {
  const declared = backend.appOrigin ?? backend.baseUrl;
  const origin = safeUrl(declared);
  if (
    !origin ||
    origin.protocol !== 'http:' ||
    origin.hostname !== '127.0.0.1' ||
    !origin.port ||
    origin.pathname !== '/' ||
    origin.search ||
    origin.hash ||
    declared.replace(/\/$/, '') !== origin.origin
  ) {
    return null;
  }
  return origin;
}

/**
 * The app a link to an app on this machine names, whatever port it carries:
 * `http://127.0.0.1:<any port>/apps/<id>/`, with nothing after the page but a
 * query or fragment. `null` for anything else.
 *
 * The port is not an identity. The daemon's port is picked fresh on every start,
 * so a link the agent wrote yesterday names a port nobody listens on today, and
 * on a shared daemon the desktop's own address is a proxy on another port. What
 * a link means is "app `<id>`", which is then opened only on this window's own
 * daemon, with a launch link minted in the main process.
 */
export function localAppPageId(candidate: string): string | null {
  if (typeof candidate !== 'string' || !LOCAL_APP_LINK.test(candidate)) return null;
  const url = safeUrl(candidate);
  if (!url) return null;
  return APP_PAGE.exec(url.pathname)?.[1] ?? null;
}

/**
 * The app a link the person asked to open in their browser names, when it is a
 * link to an app on this machine on any port: the page, or anything below it
 * (`/apps/<id>/dist/app.js`). Such a link is never handed to the browser as it
 * is. The browser holds no access cookie, so the daemon would only answer "Open
 * this app from Biorouter", and its port may be stale; the app is opened through
 * a fresh launch link instead.
 */
export function localAppLinkId(candidate: string): string | null {
  if (typeof candidate !== 'string' || !LOCAL_LINK_TO_OPEN.test(candidate)) return null;
  const url = safeUrl(candidate);
  if (!url) return null;
  return /^\/apps\/([A-Za-z0-9_-]{1,128})(?:\/.*)?$/.exec(url.pathname)?.[1] ?? null;
}

/**
 * The preview scope for opening `candidate` in this window's managed app
 * preview, or `null` when it is not a link to an app on this machine, or this
 * window has no live daemon of its own (`backend`).
 *
 * The scope is the BACKEND's app origin, not the link's: a link on any loopback
 * port is rebound to the same app on this window's daemon (`localAppPageId`).
 * That is what makes the agent's `launch_app` link, which names the shared
 * daemon's own listener, open here, and a link from an earlier run, whose port is
 * gone, too. ⚠ Only opening is rebound. Everything the preview does once open is
 * held to the scope's exact origin (`isManagedAppNavigation`,
 * `isManagedAppRequest`), so a page can never reach another port.
 */
export function managedAppPreviewScope(
  candidate: string,
  backend?: ManagedAppPreviewBackend
): ManagedAppPreviewScope | null {
  if (!backend || backend.signal.aborted) return null;
  const origin = appOriginOf(backend);
  if (!origin) return null;
  const appId = localAppPageId(candidate);
  if (!appId) return null;
  return {
    backend,
    origin: origin.origin,
    port: Number(origin.port),
    appId,
    rootPath: `/apps/${appId}/`,
  };
}

/** Whether the preview of `scope` may navigate to `candidate`: this app's page, on exactly its origin. */
export function isManagedAppNavigation(scope: ManagedAppPreviewScope, candidate: string): boolean {
  if (scope.backend.signal.aborted || typeof candidate !== 'string') return false;
  const url = safeUrl(candidate);
  if (!url || url.origin !== scope.origin || !candidate.startsWith(`${scope.origin}/`))
    return false;
  return APP_PAGE.exec(url.pathname)?.[1] === scope.appId;
}

export function isManagedAppRequest(
  scope: ManagedAppPreviewScope,
  request: { url: string; method: string; resourceType: string }
): boolean {
  if (scope.backend.signal.aborted || request.method !== 'GET') return false;
  if (request.url.startsWith('data:')) {
    return (
      (request.resourceType === 'image' && /^data:image\//i.test(request.url)) ||
      (request.resourceType === 'font' && /^data:(?:font\/|application\/font-)/i.test(request.url))
    );
  }
  const url = safeUrl(request.url);
  if (!url) return false;
  const socket = request.resourceType === 'webSocket';
  const origin = socket ? scope.origin.replace(/^http:/, 'ws:') : scope.origin;
  if (url.origin !== origin || !request.url.startsWith(`${origin}/`)) return false;
  if (socket) return url.pathname === `${scope.rootPath}agent`;
  if (request.resourceType === 'mainFrame') return isManagedAppNavigation(scope, request.url);
  if (url.pathname === `${scope.rootPath}figure-frame`) {
    return request.resourceType === 'subFrame';
  }
  if (url.pathname === scope.rootPath || url.pathname === scope.rootPath.slice(0, -1)) return true;
  if (!url.pathname.startsWith(scope.rootPath)) return false;
  const tail = url.pathname.slice(scope.rootPath.length);
  return (
    tail === 'models' ||
    tail === 'runstate' ||
    /^(?:dist|assets)\/[A-Za-z0-9_.-]+(?:\/[A-Za-z0-9_.-]+)*$/.test(tail)
  );
}

/**
 * The address the preview opens `scope`'s app at: a launch link the daemon
 * mints for a caller holding its secret (W2-HRD-1).
 *
 * An app's page, bundle and agent socket are served only to a browser holding
 * that app's access cookie (or the secret, which a page cannot send). Opening
 * the link once redeems its single-use token for the cookie inside this
 * preview's own session, and the daemon's answer moves on to the page without
 * the token; reloads and the agent socket then carry the cookie, which is why
 * "Clear site data" keeps it (`embeddedBrowser`). The link is loaded in-process
 * and never handed to another program. The link replaces
 * the address the preview was asked for, so a query or fragment on that
 * address is not kept.
 *
 * The link is minted through the backend's `baseUrl` and opened on its app
 * origin, which for a shared daemon are two different listeners (see
 * `ManagedAppPreviewBackend.appOrigin`). The token is the daemon's, so it
 * redeems on either; the page goes to the one a browser is let into. A daemon
 * that names its own listener in the answer must name this scope's, or the
 * backend moved since the preview was opened and nothing is loaded.
 *
 * The answer is checked before it is used: it must be this app's page, on this
 * backend's app origin, carrying a token and nothing else.
 */
export async function managedAppLaunchUrl(
  scope: ManagedAppPreviewScope,
  fetchImpl: typeof fetch = fetch
): Promise<string> {
  const { secretKey, signal } = scope.backend;
  if (signal.aborted) throw new Error(BACKEND_STOPPED);
  if (!secretKey) throw new Error('This app cannot be opened here.');
  const launch = await requestAppLaunch(
    scope.backend.baseUrl,
    scope.appId,
    secretKey,
    fetchImpl,
    signal
  );
  if (launch.onDaemon && new URL(launch.onDaemon).origin !== scope.origin) {
    throw new Error(BACKEND_STOPPED);
  }
  const url = `${scope.origin}${launch.path}`;
  if (!isManagedAppNavigation(scope, url)) {
    throw new Error('The app backend answered with an unexpected app address.');
  }
  return url;
}

const BACKEND_STOPPED = 'The app backend stopped. Reopen the app after it restarts.';

// An additional policy intersects with (never replaces) the server's CSP.
export const MANAGED_APP_PREVIEW_CSP = "worker-src 'none'; object-src 'none'; form-action 'none'";
