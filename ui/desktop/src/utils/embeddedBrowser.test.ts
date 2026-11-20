import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { isNavigableEmbeddedUrl } from './permissionPolicy';

/**
 * The embedded browser is the one place in this app that renders arbitrary
 * remote content, so its guards are asserted here rather than left to review.
 *
 * Most of these read the module's real source. That is deliberate: the failures
 * being guarded against are *omissions* — a handler that stops being installed,
 * a partition that reverts to the app's — and there is no value to import that
 * would reveal them. Electron is not available under vitest, so behaviour that
 * needs a live session is verified in a real Electron run instead.
 */

const source = readFileSync(join(__dirname, 'embeddedBrowser.ts'), 'utf8');
const proxySource = readFileSync(join(__dirname, 'embeddedBrowserProxy.ts'), 'utf8');
const downloadsSource = readFileSync(join(__dirname, '..', 'browser', 'downloads.ts'), 'utf8');

describe('isNavigableEmbeddedUrl', () => {
  it.each(['https://www.ucsf.edu/', 'http://example.test/path?q=1', 'https://pubmed.gov'])(
    'admits %s',
    (url) => expect(isNavigableEmbeddedUrl(url)).toBe(true)
  );

  it.each([
    'file:///etc/passwd',
    'data:text/html,<script>alert(1)</script>',
    'blob:https://example.test/id',
    'javascript:alert(1)',
    'about:blank',
    'not a url',
    '',
  ])('refuses %s', (url) => expect(isNavigableEmbeddedUrl(url)).toBe(false));

  // Credentials in a URL are a phishing primitive: the visible host is not the
  // host that gets contacted.
  it('refuses a URL carrying embedded credentials', () => {
    expect(isNavigableEmbeddedUrl('https://user:pass@evil.test/')).toBe(false);
    expect(isNavigableEmbeddedUrl('https://user@evil.test/')).toBe(false);
  });
});

describe('the embedded session is hardened, not inherited', () => {
  // The finding this whole file exists for. A handler on `defaultSession` does
  // NOT cover a partitioned session, and an unhandled session GRANTS BY
  // DEFAULT — measured with two views side by side, where the unhandled one
  // returned `granted` for notifications and allowed geolocation while the
  // app's handler was never called. Without these four lines the browser would
  // silently auto-grant notifications, geolocation, clipboard-read, media and
  // display-capture to every site the user visits.
  it.each([
    'setPermissionRequestHandler',
    'setPermissionCheckHandler',
    'setDevicePermissionHandler',
    'setDisplayMediaRequestHandler',
  ])('installs %s on its own session', (handler) => {
    expect(source).toContain(`embedded.${handler}(`);
  });

  it('uses its own partition, never the app’s', () => {
    expect(source).toContain("'persist:biorouter-embedded-browser'");
    expect(source).not.toContain("'persist:biorouter'");
    expect(source).toContain('session.fromPartition(EMBEDDED_PARTITION)');
  });

  it('never gives the remote page a preload', () => {
    // The structural guarantee: the app's IPC bridge is attached per
    // `webPreferences`, so a view constructed without one has nothing to reach.
    expect(source).not.toMatch(/preload:\s*(?!undefined)/);
    expect(source).toContain('sandbox: true');
    expect(source).toContain('contextIsolation: true');
    expect(source).toContain('nodeIntegration: false');
    // An embedded page must not be able to spawn its own guest views.
    expect(source).toContain('webviewTag: false');
  });

  it('blocks the daemon and the filesystem from the embedded partition', () => {
    // The daemon listens on loopback under a per-launch secret; a page that
    // could reach it would own the agent's tools.
    expect(source).toContain('onBeforeRequest');
    expect(source).toContain("urls: ['<all_urls>']");
    expect(source).toContain('isAllowedEmbeddedRequestUrl(details.url)');
    expect(source).toContain("proxyBypassRules: '<-loopback>'");
    expect(source).toContain("setWebRTCIPHandlingPolicy('disable_non_proxied_udp')");
    expect(proxySource).toContain('host: target.address');
    expect(proxySource).not.toContain('host: candidate');
  });

  it('opens page windows as tabs or popups only after a gesture, never as native dialogs (BB-04)', () => {
    expect(source).toContain('setWindowOpenHandler');
    // No gesture in the last five seconds: denied, with a note offering a new tab.
    expect(source).toContain('const USER_GESTURE_WINDOW_MS = 5_000;');
    expect(source).toContain('Date.now() - record.lastUserInputAt <= USER_GESTURE_WINDOW_MS');
    expect(source).toContain("action: 'deny'");
    expect(source).toContain("kind: 'popup-blocked'");
    // A tab is adopted into a view of its own; a popup is a hardened child window on the
    // browser's own partition.
    expect(source).toContain('createWindow: (options) =>');
    expect(source).toContain(
      'webPreferences: { ...HARDENED_PREFERENCES, session: embeddedSession() }'
    );
    expect(source).toContain("contents.on('did-create-window'");
    const handler = source.slice(
      source.indexOf('function installWindowOpenHandler'),
      source.indexOf('function sendToOwnerByWindow')
    );
    expect(handler).toContain("target === 'about:blank' || isNavigableEmbeddedUrl(target)");
    // A managed app preview still cannot leave its app through a window.
    expect(handler).toContain('isManagedAppNavigation(entry.managed.scope, target)');
    expect(handler).not.toContain('dialog.');
    expect(handler).not.toContain('shell.openExternal');
    // The input that counts as a gesture is the person's own (mouse, key, touch).
    expect(source).toContain("contents.on('input-event'");
  });

  it('loads sign-in pages in the browser instead of diverting them (BB-03)', () => {
    // The heuristic is only the wording of the default browser confirmation now.
    expect(source.match(/isAuthenticationNavigation\(/g)).toHaveLength(1);
    const external = source.slice(
      source.indexOf('export async function openExternalBrowserNavigation'),
      source.indexOf('function sourceRevision')
    );
    expect(external).toContain('isAuthenticationNavigation(candidate)');
    expect(source).not.toContain('requestAuthenticationConfirmation');
    const guard = source.slice(
      source.indexOf('const guardNavigation'),
      source.indexOf("contents.on('will-redirect', guardNavigation)")
    );
    expect(guard).not.toContain('isAuthenticationNavigation');
    expect(guard).toContain('isNavigableEmbeddedUrl(url)');
    expect(guard).not.toContain('shell.openExternal');
  });

  it('requires exact-host native confirmation and fresh validation for external navigation', () => {
    const confirmation = source.slice(
      source.indexOf('async function confirmPublicExternalNavigation'),
      source.indexOf('export async function openExternalBrowserNavigation')
    );
    expect(confirmation).toContain('dialog.showMessageBox');
    expect(confirmation.match(/validateExternalBrowserTarget/g)).toHaveLength(2);
    expect(confirmation).toContain('result.response !== 1');
    expect(confirmation).toContain('Destination hostname: ${validated.hostname}');
    expect(confirmation).toContain('revalidated.hostname !== validated.hostname');
  });

  it('presents a plain Chrome user agent, never Electron (BB-07)', () => {
    expect(source).toContain('embedded.setUserAgent(');
    expect(source).toContain('chromeUserAgent(process.platform, process.versions.chrome');
  });

  it('grants a site only the clipboard write its Copy button needs (BB-08)', () => {
    const handlers = source.slice(
      source.indexOf('embedded.setPermissionRequestHandler'),
      source.indexOf('embedded.setDevicePermissionHandler')
    );
    expect(handlers.match(/callback\(true\)/g)).toHaveLength(1);
    expect(handlers).toContain("permission === 'clipboard-sanitized-write'");
    expect(handlers.match(/return true/g)).toHaveLength(1);
  });

  it('never picks a client certificate and cancels HTTP authentication (BB-08)', () => {
    const guards = source.slice(
      source.indexOf('function installContentsGuards'),
      source.indexOf('function hardenPopupWindow')
    );
    expect(guards).toContain("contents.on('select-client-certificate'");
    expect(guards).toContain("contents.on('login'");
    expect(guards.match(/event\.preventDefault\(\)/g)?.length).toBeGreaterThanOrEqual(2);
    expect(guards).toContain("setWebRTCIPHandlingPolicy('disable_non_proxied_udp')");
  });

  it('admits the PDF viewer’s own resources and nothing else outside the network schemes', () => {
    const filter = source.slice(
      source.indexOf('embedded.webRequest.onBeforeRequest'),
      source.indexOf("embedded.on('will-download'")
    );
    expect(filter).toContain('isPdfViewerResourceUrl(details.url)');
    expect(filter).toContain('if (!isAllowedEmbeddedRequestUrl(details.url))');
    expect(filter).toContain('decideLocalRequest(');
    // Every branch answers the request: a lost callback would hang the page.
    expect(filter).toContain('() => callback({ cancel: true })');
  });

  it('routes every renderer-controlled system-browser open through the confirmation policy', () => {
    const main = readFileSync(join(__dirname, '..', 'main.ts'), 'utf8');
    const genericOpen = main.slice(
      main.indexOf("ipcMain.handle('open-external'"),
      main.indexOf("ipcMain.handle('directory-chooser'")
    );
    expect(genericOpen).toContain('openExternalBrowserNavigation(window, url)');
    expect(genericOpen).not.toContain('shell.openExternal');

    const legacyOpen = main.slice(
      main.indexOf("ipcMain.on('open-in-chrome'"),
      main.indexOf("ipcMain.on('restart-app'")
    );
    expect(legacyOpen).toContain('openExternalBrowserNavigation(window, url)');
    expect(legacyOpen).not.toContain('shell.openExternal');
  });

  it('routes downloads through the save dialog, and cancels one no tab owns (BB-05)', () => {
    const downloads = source.slice(
      source.indexOf("embedded.on('will-download'"),
      source.indexOf('hardenedSession = embedded;')
    );
    expect(downloads).toContain('event.preventDefault()');
    expect(downloads).toContain('handlePersonDownload(item');
    expect(downloadsSource).toContain('item.setSaveDialogOptions(');
    expect(downloadsSource).toContain("'com.apple.quarantine'");
    // The test hook never applies to a packaged app (testHooks.ts).
    expect(downloadsSource).toContain('if (testDownloadDir)');
  });
});

describe('the panel keeps the native view honest', () => {
  // A native view paints above the DOM with no shared stacking context, so it
  // has to be told to hide. Losing this makes the view cover modals and menus.
  it('can be hidden independently of its bounds', () => {
    expect(source).toContain('export function setEmbeddedBrowserVisible');
    expect(source).toContain('entry.view.setVisible(visible)');
  });

  it('reports an empty capture rather than an empty PNG', () => {
    // capturePage returns a zero-byte image for a hidden-then-navigated view
    // and does not reject, so the empty case has to be checked explicitly.
    expect(source).toContain('image.isEmpty()');
    expect(source).toContain('revision !== sourceRevision(entry)');
  });

  it('revisions committed navigation and never returns a mixed-document text snapshot', () => {
    expect(source).toContain("contents.on('did-navigate'");
    expect(source).toContain("contents.on('did-navigate-in-page'");
    expect(source).toContain('entry.revision += 1');
    expect(source).toContain('if (contents.isLoadingMainFrame()) return null');
    expect(source).toContain('revision !== entry.revision || url !== contents.getURL()');
    expect(source).toContain('sourceRevision: sourceRevision(entry)');
    expect(source).not.toMatch(/executeJavaScript\([\s\S]{0,500},\s*true\s*\)/);
  });

  it('tears views down with the window that owns them', () => {
    // The view is a child of the window's contentView, not of the React tree,
    // so nothing in the renderer unmounts it.
    expect(source).toContain('export function destroyEmbeddedBrowsersForWindow');
    const main = readFileSync(join(__dirname, '..', 'main.ts'), 'utf8');
    expect(main).toContain('destroyEmbeddedBrowsersForWindow(mainWindow)');
  });

  it('resolves the owning window from the event sender, and only its own renderer', () => {
    // Otherwise one window could drive another window's view, or a popup's page could.
    const main = readFileSync(join(__dirname, '..', 'main.ts'), 'utf8');
    const owner = main.slice(
      main.indexOf('function browserOwnerWindow'),
      main.indexOf('function isViewId')
    );
    expect(owner).toContain('BrowserWindow.fromWebContents(event.sender)');
    expect(owner).toContain('window.webContents === event.sender');
    const handlers = main.slice(
      main.indexOf('function isViewId'),
      main.indexOf("ipcMain.handle('read-artifact-file'")
    );
    const channels = handlers.match(/ipcMain\.handle\(\s*'embedded-browser:[a-z-]+'/g) ?? [];
    expect(channels.length).toBeGreaterThanOrEqual(20);
    expect(handlers.match(/browserOwnerWindow\(event\)/g)?.length).toBe(channels.length);
    expect(source).toContain('views.get(viewKey(window, viewId))');
  });

  it('keeps a tab when the panel hides it, and ends it only on destroy (BB-01)', () => {
    const release = source.slice(
      source.indexOf('export function releaseEmbeddedBrowser'),
      source.indexOf('export function claimEmbeddedBrowser')
    );
    expect(release).toContain('entry.view.setVisible(false)');
    expect(release).not.toContain('close()');
    // Attaching to a tab that exists keeps its page.
    const create = source.slice(
      source.indexOf('export function createEmbeddedBrowser'),
      source.indexOf('if (!isNavigableEmbeddedUrl(initialUrl)) return null;')
    );
    expect(create).toContain('existing.onState = onState');
    expect(create).toContain('return readState(existing)');
    // A tab a page opened closes if no panel claims it.
    expect(source).toContain('CLAIM_TIMEOUT_MS');
  });

  it('answers display-capture denial instead of leaving the page pending', () => {
    expect(source).toContain('setDisplayMediaRequestHandler((_request, callback) => callback({}))');
  });
});
