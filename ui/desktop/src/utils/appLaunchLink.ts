/**
 * An app's one-time launch link (W2-HRD-1), minted by the daemon for a caller
 * holding its secret.
 *
 * The daemon serves an app's page only to a browser holding that app's access
 * cookie, and opening this link once sets it. Whoever opens the link FIRST gets
 * the app, so it must never go on another program's command line: the desktop
 * hands it to the system browser through a private page
 * (`appBrowserLaunch`), and a `biorouter serve` tab, which already is the
 * browser, opens it itself (`renderer.tsx`). This module has no Node imports so
 * both can use it.
 *
 * Where the link is MINTED and where the browser is SENT are two questions
 * ({@link AppLaunch}). The desktop asks through the address its window talks to,
 * which for a shared daemon is a local proxy that only the desktop can use, and
 * sends the browser to the daemon's own listener.
 */

/** How long to wait for the daemon to mint a launch link. */
const LAUNCH_TIMEOUT_MS = 15_000;

const APP_ID = /^[A-Za-z0-9_-]{1,128}$/;

const UNEXPECTED = 'The app backend answered with an unexpected app address.';

/** The hosts a launch link on the daemon's own listener may name: this machine's. */
const LOOPBACK_HOSTS = new Set(['127.0.0.1', 'localhost', '[::1]']);

/**
 * The daemon's base address without a trailing slash: an http(s) URL with no
 * credentials, query or fragment. A path is kept, for a daemon served below
 * one.
 */
function daemonBase(baseUrl: string): string {
  let url: URL;
  try {
    url = new URL(baseUrl);
  } catch {
    throw new Error('The app backend has no usable address.');
  }
  if (
    (url.protocol !== 'http:' && url.protocol !== 'https:') ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  ) {
    throw new Error('The app backend has no usable address.');
  }
  return `${url.origin}${url.pathname.replace(/\/+$/, '')}`;
}

/**
 * A launch link the daemon minted, on the two addresses it can be opened at.
 * `path` is the page alone (`/apps/<id>/?t=<token>`); the token is the daemon's,
 * so it redeems on any listener of that daemon.
 *
 * `viaBase` is the page on the address the caller reached the daemon at. That is
 * the right one for a caller the browser shares an address with (a `biorouter
 * serve` tab) and for a daemon the desktop does not run (an external backend,
 * possibly through a tunnel, whose own listener this machine cannot reach).
 *
 * `onDaemon` is the same page on the daemon's OWN listener, when the daemon
 * named one this machine's browser may be sent to. The desktop needs it: it
 * reaches a shared daemon through a local proxy that admits only requests
 * carrying the desktop's secret, so a link on the proxy's origin answered the
 * browser with an empty 403 ("Access to 127.0.0.1 was denied"). An older daemon
 * names no listener, and then this is `undefined`.
 */
export type AppLaunch = { path: string; viaBase: string; onDaemon?: string };

/** The proxy's refusal codes (`daemonRuntime.ts`), whose `message` is written for a person. */
const PROXY_REFUSALS = new Set(['daemon_restarted', 'daemon_unavailable']);

/** The sentence for a launch the daemon (or the proxy in front of it) refused with `response`. */
async function refusalSentence(response: Response): Promise<string> {
  if (response.status === 404) return 'This app no longer exists.';
  if (response.status === 401 || response.status === 403) {
    return "Biorouter's background service refused this window's connection. Quit and reopen Biorouter.";
  }
  if (response.status === 502) {
    const body = (await response.json().catch(() => null)) as {
      code?: unknown;
      message?: unknown;
    } | null;
    if (
      typeof body?.code === 'string' &&
      PROXY_REFUSALS.has(body.code) &&
      typeof body.message === 'string' &&
      body.message.trim()
    ) {
      return body.message.trim().slice(0, 300);
    }
  }
  return 'The app backend would not open this app.';
}

/**
 * The daemon's own address for the launch link `linkPath`, from the `url` it
 * answered with: `undefined` when it named none, or named one that is not a
 * plain-http loopback address on an explicit port (a daemon bound to a LAN
 * address, or a value nobody should be sent to). Anything else about it must be
 * exactly `linkPath`, token included, or nothing is opened.
 */
function daemonLink(url: unknown, linkPath: string, appId: string): string | undefined {
  if (url === undefined || url === null) return undefined;
  if (typeof url !== 'string') throw new Error(UNEXPECTED);
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw new Error(UNEXPECTED);
  }
  if (
    parsed.protocol !== 'http:' ||
    !LOOPBACK_HOSTS.has(parsed.hostname) ||
    !parsed.port ||
    parsed.username ||
    parsed.password
  ) {
    return undefined;
  }
  // Spelled exactly as the page it was minted with: no fragment, no other query,
  // no other app, and the same token.
  if (parsed.pathname !== `/apps/${appId}/` || url !== `${parsed.origin}${linkPath}`) {
    throw new Error(UNEXPECTED);
  }
  return url;
}

/**
 * Ask the daemon at `baseUrl`, as a caller holding `secretKey`, for a one-time
 * launch link to app `appId` ({@link AppLaunch}). The answer must be exactly that
 * app's page carrying a token, or nothing is opened. `signal` ends the wait
 * early, as the timeout does.
 */
export async function requestAppLaunch(
  baseUrl: string,
  appId: string,
  secretKey: string,
  fetchImpl: typeof fetch = fetch,
  signal?: AbortSignal
): Promise<AppLaunch> {
  if (!APP_ID.test(appId)) throw new Error('That is not an app name.');
  if (!secretKey) throw new Error('This app cannot be opened from here.');
  const base = daemonBase(baseUrl);
  let response: Response;
  try {
    response = await fetchImpl(`${base}/apps/${appId}/launch`, {
      method: 'POST',
      headers: { 'X-Secret-Key': secretKey },
      redirect: 'error',
      signal: signal
        ? AbortSignal.any([signal, AbortSignal.timeout(LAUNCH_TIMEOUT_MS)])
        : AbortSignal.timeout(LAUNCH_TIMEOUT_MS),
    });
  } catch {
    throw new Error('The app backend did not answer. Try opening the app again.');
  }
  if (!response.ok) throw new Error(await refusalSentence(response));
  const body = (await response.json().catch(() => null)) as {
    path?: unknown;
    url?: unknown;
  } | null;
  const linkPath = body?.path;
  const expected = new RegExp(`^/apps/${appId}/\\?t=[0-9a-f]{64}$`);
  if (typeof linkPath !== 'string' || !expected.test(linkPath)) {
    throw new Error(UNEXPECTED);
  }
  return {
    path: linkPath,
    viaBase: `${base}${linkPath}`,
    onDaemon: daemonLink(body?.url, linkPath, appId),
  };
}

/**
 * The one-time launch link for app `appId` on the address the caller reached the
 * daemon at ({@link AppLaunch.viaBase}): what a tab that is itself the browser
 * opens.
 */
export async function mintAppLaunchLink(
  baseUrl: string,
  appId: string,
  secretKey: string,
  fetchImpl: typeof fetch = fetch,
  signal?: AbortSignal
): Promise<string> {
  return (await requestAppLaunch(baseUrl, appId, secretKey, fetchImpl, signal)).viaBase;
}

/**
 * Which address of `launch` the system browser is sent to.
 *
 * With `daemonListener` (Biorouter runs the daemon, shared or per window), the
 * daemon's own listener: the address the daemon named, or else `appOrigin`, the
 * listener the main process learned when the window attached (a daemon from
 * before the answer named one). Without it (an external backend, which may be
 * reached through a tunnel whose far end this machine cannot see), the address
 * the caller reached the daemon at.
 */
export function browserLaunchLink(
  launch: AppLaunch,
  { daemonListener, appOrigin }: { daemonListener: boolean; appOrigin?: string }
): string {
  if (!daemonListener) return launch.viaBase;
  if (launch.onDaemon) return launch.onDaemon;
  return appOrigin && LOOPBACK_ORIGIN.test(appOrigin)
    ? `${appOrigin}${launch.path}`
    : launch.viaBase;
}

/** A plain-http loopback origin on an explicit port, and nothing else. */
const LOOPBACK_ORIGIN = /^http:\/\/(?:127\.0\.0\.1|localhost|\[::1\]):\d{1,5}$/;

type LaunchTab = Pick<Window, 'close'> & {
  opener: unknown;
  location: Pick<Location, 'replace'>;
};

/**
 * Open app `appId` from a tab that is itself the browser (`biorouter serve`).
 *
 * There is no command line here to keep the link off, so the link goes
 * straight to a new tab. That tab is opened inside the click, before the daemon
 * is asked, so a popup blocker lets it through, and it is cut off from this
 * page before it is sent on.
 */
export async function openAppInThisBrowser(
  baseUrl: string,
  appId: string,
  secretKey: string,
  {
    openTab = () => window.open('about:blank', '_blank'),
    fetchImpl = fetch,
  }: { openTab?: () => LaunchTab | null; fetchImpl?: typeof fetch } = {}
): Promise<void> {
  const tab = openTab();
  if (!tab) throw new Error('The browser did not open a new tab.');
  try {
    const link = await mintAppLaunchLink(baseUrl, appId, secretKey, fetchImpl);
    tab.opener = null;
    tab.location.replace(link);
  } catch (error) {
    tab.close();
    throw error;
  }
}
