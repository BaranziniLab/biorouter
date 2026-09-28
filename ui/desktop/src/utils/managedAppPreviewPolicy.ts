import { mintAppLaunchLink } from './appLaunchLink';

export type ManagedAppPreviewBackend = {
  baseUrl: string;
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

export function managedAppPreviewScope(
  candidate: string,
  backend?: ManagedAppPreviewBackend
): ManagedAppPreviewScope | null {
  if (!backend || backend.signal.aborted) return null;
  const base = safeUrl(backend.baseUrl);
  if (
    !base ||
    base.protocol !== 'http:' ||
    base.hostname !== '127.0.0.1' ||
    !base.port ||
    base.pathname !== '/' ||
    base.search ||
    base.hash ||
    backend.baseUrl.replace(/\/$/, '') !== base.origin
  ) {
    return null;
  }
  const url = safeUrl(candidate);
  if (!url || url.origin !== base.origin || !candidate.startsWith(`${base.origin}/`)) return null;
  const match = /^\/apps\/([A-Za-z0-9_-]{1,128})\/?$/.exec(url.pathname);
  if (!match) return null;
  return {
    backend,
    origin: base.origin,
    port: Number(base.port),
    appId: match[1],
    rootPath: `/apps/${match[1]}/`,
  };
}

export function isManagedAppNavigation(scope: ManagedAppPreviewScope, candidate: string): boolean {
  return managedAppPreviewScope(candidate, scope.backend)?.appId === scope.appId;
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
 * The answer is checked before it is used: it must be this app's page, on this
 * backend's origin, carrying a token and nothing else.
 */
export async function managedAppLaunchUrl(
  scope: ManagedAppPreviewScope,
  fetchImpl: typeof fetch = fetch
): Promise<string> {
  const { secretKey, signal } = scope.backend;
  if (signal.aborted) throw new Error('The app backend stopped. Reopen the app after it restarts.');
  if (!secretKey) throw new Error('This app cannot be opened here.');
  const url = await mintAppLaunchLink(scope.origin, scope.appId, secretKey, fetchImpl, signal);
  if (!isManagedAppNavigation(scope, url)) {
    throw new Error('The app backend answered with an unexpected app address.');
  }
  return url;
}

// An additional policy intersects with (never replaces) the server's CSP.
export const MANAGED_APP_PREVIEW_CSP = "worker-src 'none'; object-src 'none'; form-action 'none'";
