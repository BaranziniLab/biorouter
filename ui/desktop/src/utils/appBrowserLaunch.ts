import { randomBytes } from 'node:crypto';
import { promises as fs } from 'node:fs';
import path from 'node:path';

/**
 * Opening a built app in the system browser (W2-HRD-1), from the main process.
 *
 * The daemon serves an app's page only to a browser holding that app's access
 * cookie, set by opening a one-time launch link that it hands out to a caller
 * holding its secret. Whoever opens the link FIRST gets the app, and a command
 * line is readable by every account on the machine (`ps` on macOS,
 * `/proc/<pid>/cmdline` on Linux): `shell.openExternal` runs `xdg-open <url>` on
 * Linux and puts the URL on the browser's command line on Windows, so a
 * co-tenant polling for `?t=` would redeem the link before the browser started,
 * keep the app's cookie for the daemon's run, and leave the owner looking at a
 * refusal. So the link is never handed to an opener. It is written into a page
 * only this account can read, which sends the browser on to it, and the page is
 * what gets opened; Jupyter hands its token over the same way. The daemon answers
 * the link with a page of its own that sets the cookie, because a navigation a
 * `file:` page starts would not carry a `SameSite=Strict` cookie across a
 * redirect.
 *
 * The renderer names only the app. The link is minted here, with the secret,
 * and never crosses into the renderer.
 */

/** How long the main process waits for the daemon to mint a launch link. */
const LAUNCH_TIMEOUT_MS = 15_000;

/** A page this old holds an expired link: the daemon's last five minutes. */
export const LAUNCH_PAGE_STALE_MS = 10 * 60_000;

const APP_ID = /^[A-Za-z0-9_-]{1,128}$/;
const LAUNCH_PAGE = /^launch-[0-9a-f]{32}\.html$/;

/** The daemon's origin, from the base URL the main process holds for it. */
function daemonOrigin(baseUrl: string): string {
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
    url.hash ||
    (url.pathname !== '/' && url.pathname !== '')
  ) {
    throw new Error('The app backend has no usable address.');
  }
  return url.origin;
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
  const origin = daemonOrigin(baseUrl);
  let response: Response;
  try {
    response = await fetchImpl(`${origin}/apps/${appId}/launch`, {
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
  return `${origin}${linkPath}`;
}

function htmlAttribute(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

/**
 * A folder for launch pages that this account alone can enter: created 0700,
 * refused if it is a link or another account's, and tightened if it is ours
 * and looser. (Windows keeps the profile's own access list.)
 */
async function privateFolder(directory: string): Promise<void> {
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  const folder = await fs.lstat(directory);
  if (!folder.isDirectory() || folder.isSymbolicLink()) {
    throw new Error('The folder for launch pages is not a plain folder.');
  }
  if (process.platform === 'win32') return;
  if (typeof process.getuid === 'function' && folder.uid !== process.getuid()) {
    throw new Error('The folder for launch pages belongs to another account.');
  }
  if ((folder.mode & 0o077) !== 0) await fs.chmod(directory, 0o700);
}

/** Remove launch pages whose links expired long ago. Best effort. */
async function removeStaleLaunchPages(directory: string, now: number): Promise<void> {
  const names = await fs.readdir(directory).catch(() => [] as string[]);
  await Promise.all(
    names
      .filter((name) => LAUNCH_PAGE.test(name))
      .map(async (name) => {
        const page = path.join(directory, name);
        const stat = await fs.lstat(page).catch(() => null);
        if (stat?.isFile() && now - stat.mtimeMs >= LAUNCH_PAGE_STALE_MS) {
          await fs.unlink(page).catch(() => {});
        }
      })
  );
}

/**
 * Write `link` into a page in `directory` that only this account can read and
 * that sends the browser on to it, and answer the page's path: the only thing
 * an opener is handed.
 */
export async function writeAppLaunchPage(
  directory: string,
  link: string,
  now: number = Date.now()
): Promise<string> {
  await privateFolder(directory);
  await removeStaleLaunchPages(directory, now);
  const target = htmlAttribute(link);
  const page = path.join(directory, `launch-${randomBytes(16).toString('hex')}.html`);
  await fs.writeFile(
    page,
    '<!doctype html><meta charset=utf-8><meta name=referrer content=no-referrer>' +
      `<meta http-equiv="refresh" content="0;url=${target}">` +
      '<title>Opening Biorouter app</title>' +
      `<p>Opening the app. If nothing happens, <a href="${target}">open it here</a>. ` +
      'The address works once.</p>\n',
    { mode: 0o600, flag: 'wx' }
  );
  return page;
}

export type OpenAppInSystemBrowserOptions = {
  baseUrl: string;
  appId: string;
  secretKey: string;
  /** Where launch pages are written: a folder of this app's own user data. */
  directory: string;
  /** `shell.openPath`: resolves to an error message, or '' once opened. */
  openPath: (page: string) => Promise<string>;
  fetchImpl?: typeof fetch;
};

/** Open app `appId` in the system browser, handing the opener a launch page. */
export async function openAppInSystemBrowser(
  options: OpenAppInSystemBrowserOptions
): Promise<void> {
  const link = await mintAppLaunchLink(
    options.baseUrl,
    options.appId,
    options.secretKey,
    options.fetchImpl
  );
  const page = await writeAppLaunchPage(options.directory, link);
  const failure = await options.openPath(page);
  if (failure) throw new Error(`Your browser could not be opened: ${failure}`);
}
