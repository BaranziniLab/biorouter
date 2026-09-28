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
 */

/** How long to wait for the daemon to mint a launch link. */
const LAUNCH_TIMEOUT_MS = 15_000;

const APP_ID = /^[A-Za-z0-9_-]{1,128}$/;

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
 * The one-time launch link for app `appId`, minted by the daemon at `baseUrl`
 * for a caller holding `secretKey`. The answer must be exactly that app's page
 * carrying a token, or nothing is opened.
 */
export async function mintAppLaunchLink(
  baseUrl: string,
  appId: string,
  secretKey: string,
  fetchImpl: typeof fetch = fetch
): Promise<string> {
  if (!APP_ID.test(appId)) throw new Error('That is not an app name.');
  if (!secretKey) throw new Error('This app cannot be opened from here.');
  const base = daemonBase(baseUrl);
  let response: Response;
  try {
    response = await fetchImpl(`${base}/apps/${appId}/launch`, {
      method: 'POST',
      headers: { 'X-Secret-Key': secretKey },
      redirect: 'error',
      signal: AbortSignal.timeout(LAUNCH_TIMEOUT_MS),
    });
  } catch {
    throw new Error('The app backend did not answer. Try opening the app again.');
  }
  if (response.status === 404) throw new Error('This app no longer exists.');
  if (!response.ok) throw new Error('The app backend would not open this app.');
  const body = (await response.json().catch(() => null)) as { path?: unknown } | null;
  const linkPath = body?.path;
  const expected = new RegExp(`^/apps/${appId}/\\?t=[0-9a-f]{64}$`);
  if (typeof linkPath !== 'string' || !expected.test(linkPath)) {
    throw new Error('The app backend answered with an unexpected app address.');
  }
  return `${base}${linkPath}`;
}

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
