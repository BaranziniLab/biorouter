import { execFile, spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { promises as fs } from 'node:fs';
import path from 'node:path';
import { browserLaunchLink, requestAppLaunch } from './appLaunchLink';

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
 * The renderer names only the app. The link is minted here, with the secret
 * (`appLaunchLink`), and never crosses into the renderer.
 */

/** A page this old holds an expired link: the daemon's last five minutes. */
export const LAUNCH_PAGE_STALE_MS = 10 * 60_000;

const LAUNCH_PAGE = /^launch-[0-9a-f]{32}\.html$/;

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

/** What {@link openPageInDefaultBrowser} needs from Electron and the OS, injected so it is testable. */
export type DefaultBrowserOpener = {
  platform: typeof process.platform;
  /** `app.getApplicationInfoForProtocol`: the application that opens `url`'s scheme. */
  applicationForProtocol: (url: string) => Promise<{ path: string }>;
  /** Run `file` with `args` (never through a shell) and answer its standard output. */
  run: (file: string, args: string[]) => Promise<string>;
  /**
   * Start `file` with `args` (never through a shell) and leave it running: resolves once it has
   * started. For a program that IS the browser, which must not be waited on.
   */
  launch: (file: string, args: string[]) => Promise<void>;
  /** `shell.openPath`: resolves to an error message, or '' once opened. */
  openPath: (page: string) => Promise<string>;
};

/** How long an opener may take to hand the page over. */
const OPENER_TIMEOUT_MS = 10_000;

/**
 * Run `file` with `args`, never through a shell, and answer its standard output:
 * {@link DefaultBrowserOpener.run} for the real platform.
 */
export function runFile(file: string, args: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(file, args, { timeout: OPENER_TIMEOUT_MS, windowsHide: true }, (error, stdout) =>
      error ? reject(error) : resolve(String(stdout))
    );
  });
}

/**
 * Start `file` with `args`, never through a shell, detached from this process:
 * {@link DefaultBrowserOpener.launch} for the real platform.
 *
 * ⚠ Not {@link runFile}. That waits for the program to exit and kills it at its timeout, which for
 * a browser that was not already running is the browser, ten seconds after it opened. And no
 * `windowsHide: true`: on Windows it also asks a GUI program's first window to start hidden.
 */
export function launchFile(file: string, args: string[]): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = spawn(file, args, { detached: true, stdio: 'ignore', windowsHide: false });
    child.once('error', reject);
    child.once('spawn', () => {
      child.unref();
      resolve();
    });
  });
}

/** A desktop file id as `xdg-settings` prints it, and nothing that could be read as an option. */
const DESKTOP_ID = /^[A-Za-z0-9][A-Za-z0-9._-]*\.desktop$/;

/**
 * The desktop file id snapd gives a snap's application: `<snap>_<app>.desktop`
 * (`firefox_firefox.desktop`, `chromium_chromium.desktop`). A browser packaged any other way
 * has a plain name (`firefox.desktop`, `google-chrome.desktop`) or, from Flatpak, a reverse-DNS
 * one (`org.mozilla.firefox.desktop`).
 */
const SNAP_DESKTOP_ID = /^[a-z0-9][a-z0-9-]*_[A-Za-z0-9][A-Za-z0-9-]*\.desktop$/;

/** An absolute path to a Windows program: what `applicationForProtocol` names there. */
const WINDOWS_PROGRAM = /^(?:[A-Za-z]:\\|\\\\)[^"<>|?*\r\n]+\.exe$/i;

/** The person's default web browser, as {@link openPageInBrowser} reaches it. */
export type DefaultBrowser =
  /** macOS and Windows: the application that opens `https:` links, by its path. */
  | { kind: 'application'; path: string }
  /** Linux: the desktop's default web browser, by its desktop file id. */
  | { kind: 'desktop'; id: string }
  /**
   * Linux: a snap, which cannot read a launch page. A snap's sandbox lets it read the person's
   * files only outside hidden folders, and launch pages live in this app's own data folder
   * (`~/.config/Biorouter`). Rather than make a visible folder in the person's home for them,
   * the person is offered the link to copy (`OpenAppInSystemBrowserOptions.copyLink`). Stock
   * Ubuntu's Firefox and Chromium are snaps. (A Flatpak browser is not one of these: the desktop
   * file Flatpak exports runs it with `--file-forwarding`, which hands it the page through the
   * document portal.)
   */
  | { kind: 'confined'; id: string }
  /** Nothing better was found: the generic opener. */
  | { kind: 'generic' };

/**
 * Find the person's default WEB BROWSER: the program that opens `https:` links.
 *
 * Not whatever opens `.html` files, which is what `shell.openPath` uses. On a developer's machine
 * that is often an editor (VS Code, BBEdit, Xcode), which showed the launch page's source, never
 * redeemed its link, and reported success. `biorouter apps open` asks for the browser the same
 * way.
 *
 * - macOS and Windows: the `https:` handler the system names.
 * - Linux: the desktop's default web browser (`xdg-settings`).
 * - Anything else, or a failure: the generic opener. On Linux that is `xdg-open`, which honours
 *   `$BROWSER` where there is no desktop to ask.
 */
export async function findDefaultBrowser(opener: DefaultBrowserOpener): Promise<DefaultBrowser> {
  try {
    if (opener.platform === 'darwin' || opener.platform === 'win32') {
      const program = (await opener.applicationForProtocol('https://'))?.path;
      if (program && (opener.platform === 'darwin' || WINDOWS_PROGRAM.test(program))) {
        return { kind: 'application', path: program };
      }
    } else if (opener.platform === 'linux') {
      const id = (await opener.run('xdg-settings', ['get', 'default-web-browser'])).trim();
      if (DESKTOP_ID.test(id)) {
        return SNAP_DESKTOP_ID.test(id) ? { kind: 'confined', id } : { kind: 'desktop', id };
      }
    }
  } catch {
    // Whatever the platform's own way failed on, the generic opener may not.
  }
  return { kind: 'generic' };
}

/** What a browser that cannot read a launch page is answered with, when it is asked to. */
export const CONFINED_BROWSER_MESSAGE =
  'your default browser is a snap, which cannot read the page Biorouter hands it.';

/**
 * Open the launch page `page` in `browser`, and answer as `shell.openPath` does: '' once it is
 * opened, else what went wrong.
 *
 * - macOS: through `open -a`, a LaunchServices call that starts no process whose arguments
 *   could be read.
 * - Windows: the browser's program, started with the page as its argument and left running.
 * - Linux: `gtk-launch`, which hands the page to the desktop file and returns.
 * - When any of those fails, or no browser was found: `shell.openPath`.
 *
 * ⚠ Only the page's PATH is ever an argument. The link inside it opens the app for whoever
 * redeems it first, and every argument here is readable by every account on the machine (see
 * this module's header).
 */
export async function openPageInBrowser(
  page: string,
  browser: DefaultBrowser,
  opener: DefaultBrowserOpener
): Promise<string> {
  try {
    if (browser.kind === 'application' && opener.platform === 'darwin') {
      await opener.run('/usr/bin/open', ['-a', browser.path, page]);
      return '';
    }
    if (browser.kind === 'application' && opener.platform === 'win32') {
      await opener.launch(browser.path, [page]);
      return '';
    }
    if (browser.kind === 'desktop') {
      await opener.run('gtk-launch', [browser.id, page]);
      return '';
    }
    // The generic opener would start the same browser, which would show "Access denied" while
    // this reported success.
    if (browser.kind === 'confined') return CONFINED_BROWSER_MESSAGE;
  } catch {
    // Whatever the platform's own way failed on, the generic opener may not.
  }
  return opener.openPath(page);
}

/** {@link findDefaultBrowser}, then {@link openPageInBrowser}. */
export async function openPageInDefaultBrowser(
  page: string,
  opener: DefaultBrowserOpener
): Promise<string> {
  return openPageInBrowser(page, await findDefaultBrowser(opener), opener);
}

export type OpenAppInSystemBrowserOptions = {
  /** Where the daemon is asked for the link, with `secretKey`: the address this window talks to. */
  baseUrl: string;
  appId: string;
  secretKey: string;
  /**
   * Send the browser to the daemon's own listener rather than to `baseUrl`:
   * true when Biorouter runs the daemon (`browserLaunchLink`).
   */
  daemonListener: boolean;
  /** The daemon's own listener as learned when the window attached, for a daemon too old to name it. */
  appOrigin?: string;
  /** Where launch pages are written: a folder of this app's own user data. */
  directory: string;
  /** Opens a launch page (`openPageInBrowser`): '' once opened, else what went wrong. */
  openPage: (page: string) => Promise<string>;
  /**
   * Present when the default browser cannot read a launch page (`DefaultBrowser` `confined`): the
   * person is offered the link to paste into it instead, and no page is written or opened.
   * `confirm` asks first and answers whether to go ahead, so a link is minted only for a yes and
   * its five minutes start then. `copy` hands it over (the clipboard), which, like the launch
   * page, only this account's own programs can read.
   */
  copyLink?: {
    confirm: () => Promise<boolean>;
    copy: (link: string) => void;
  };
  fetchImpl?: typeof fetch;
};

/**
 * Open app `appId` in the system browser, handing the opener a launch page, or with
 * `copyLink`, handing the person the link.
 */
export async function openAppInSystemBrowser(
  options: OpenAppInSystemBrowserOptions
): Promise<void> {
  if (options.copyLink && !(await options.copyLink.confirm())) return;
  const launch = await requestAppLaunch(
    options.baseUrl,
    options.appId,
    options.secretKey,
    options.fetchImpl
  );
  const link = browserLaunchLink(launch, {
    daemonListener: options.daemonListener,
    appOrigin: options.appOrigin,
  });
  if (options.copyLink) {
    options.copyLink.copy(link);
    return;
  }
  const page = await writeAppLaunchPage(options.directory, link);
  const failure = await options.openPage(page);
  if (failure) throw new Error(`Your browser could not be opened: ${failure}`);
}
