import {
  app,
  BrowserWindow,
  dialog,
  WebContentsView,
  session,
  shell,
  type Session,
} from 'electron';
import log from 'electron-log';
import { isAllowedEmbeddedRequestUrl, isAuthenticationNavigation } from './embeddedBrowserPolicy';
import { startEmbeddedBrowserProxy, stopEmbeddedBrowserProxy } from './embeddedBrowserProxy';
import { validateExternalBrowserTarget } from './externalBrowserNavigation';
import { isNavigableEmbeddedUrl } from './permissionPolicy';
import {
  isManagedAppNavigation,
  managedAppLaunchUrl,
  managedAppPreviewScope,
  type ManagedAppPreviewBackend,
} from './managedAppPreviewPolicy';
import { managedAppSession, type ManagedAppPreviewSession } from './managedAppPreviewSession';
import { PREVIEW_ACTIVITY_IDLE, PREVIEW_ACTIVITY_INSTALL } from './previewActivity';

/**
 * A real, interactive browser inside the artifact panel.
 *
 * **Why a `WebContentsView` and not an `<iframe>`.** Half of the sites this
 * audience actually opens refuse to be framed — `google.com`, PubMed, NCBI
 * Gene, ClinicalTrials.gov and Nature all send `X-Frame-Options` or
 * `frame-ancestors`, while `ucsf.edu` does not. An iframe implementation
 * therefore demos perfectly and fails on the destinations that matter. It is
 * also unfixable by stripping those headers: a cross-site frame is a
 * third-party context where `SameSite=Lax` cookies are not sent, so the page
 * would render logged-out and half-broken, and clickjacking protection would be
 * gone for nothing. A `WebContentsView` is a top-level browsing context, so
 * those headers do not apply and the page behaves exactly as it does in Chrome
 * — clicking, typing, scrolling, JS and ordinary site cookies all work.
 * Authentication navigations leave for the system browser because this
 * isolated partition cannot safely or reliably share their callback state.
 *
 * **What it costs.** A native view paints above the DOM. It does not respect
 * stacking, scrolling, border radius or modals, so the renderer must tell us
 * where it goes and hide it whenever something needs to sit on top. That bill
 * is the price of the paragraph above, and it is paid in `setBounds`/`setVisible`.
 */

/**
 * A partition of its own — **never** the app's `persist:biorouter`.
 *
 * Sharing the app's partition would put arbitrary third-party cookies, storage
 * and service workers in the same jar as BioRouter's own origin, and would let
 * an embedded page reach the daemon's session.
 */
const EMBEDDED_PARTITION = 'persist:biorouter-embedded-browser';

export type EmbeddedBrowserBounds = { x: number; y: number; width: number; height: number };

export type EmbeddedBrowserState = {
  url: string;
  title: string;
  managedApp?: boolean;
  /** Changes whenever the top-level document or SPA route commits. */
  sourceRevision: string;
  canGoBack: boolean;
  canGoForward: boolean;
  isLoading: boolean;
  error: string | null;
};

type Entry = {
  view: WebContentsView;
  window: BrowserWindow;
  visible: boolean;
  bounds: EmbeddedBrowserBounds;
  revision: number;
  requestAuthenticationConfirmation: (url: string) => void;
  managed?: ManagedAppPreviewSession;
  releaseManagedView?: () => void;
};

const views = new Map<string, Entry>();
const MAX_PAGE_TITLE_CHARS = 512;
const MAX_PAGE_URL_CHARS = 8 * 1024;
const MAX_PAGE_ERROR_CHARS = 1024;
let hardenedSession: Session | null = null;
let embeddedNetworkReady: Promise<void> | null = null;
let proxyTeardownRegistered = false;
const registeredOwnerTeardown = new WeakSet<BrowserWindow>();

function viewKey(window: BrowserWindow, viewId: string): string {
  return `${window.webContents.id}:${viewId}`;
}

function entryFor(window: BrowserWindow, viewId: string): Entry | undefined {
  return views.get(viewKey(window, viewId));
}

function prepareEmbeddedNetwork(embedded: Session): Promise<void> {
  if (embeddedNetworkReady) return embeddedNetworkReady;
  const operation = startEmbeddedBrowserProxy().then(async (port) => {
    await embedded.setProxy({
      proxyRules: `socks5://127.0.0.1:${port}`,
      // Chromium bypasses proxies for loopback unless this subtracts the
      // implicit bypass list. The proxy itself is the component that rejects
      // loopback/private targets after resolving and pins the public IP socket.
      proxyBypassRules: '<-loopback>',
    });
  });
  embeddedNetworkReady = operation;
  void operation.catch(() => {
    if (embeddedNetworkReady === operation) embeddedNetworkReady = null;
    void stopEmbeddedBrowserProxy();
  });
  if (!proxyTeardownRegistered) {
    proxyTeardownRegistered = true;
    app.once('before-quit', () => {
      void stopEmbeddedBrowserProxy();
    });
  }
  return operation;
}

/**
 * Locks down the embedded partition.
 *
 * ⚠ **This is the single most important function in this file, and the reason
 * it exists at all.** A permission handler installed on `session.defaultSession`
 * — which is where the app installs its own — **does not cover a partitioned
 * session, and an unhandled session grants by default.** Measured with two
 * views side by side: the one whose partition had no handler returned
 * `granted` for `Notification.requestPermission()` and allowed geolocation,
 * while the app's handler was never called once.
 *
 * So an embedded browser that simply inherited the app's setup would silently
 * auto-grant notifications, geolocation, clipboard-read, media and
 * display-capture to every site the user visited.
 *
 * Both handlers are needed: `setPermissionRequestHandler` covers asynchronous
 * prompts, `setPermissionCheckHandler` covers the synchronous checks behind
 * `navigator.permissions.query` and device enumeration.
 */
function embeddedSession(): Session {
  if (hardenedSession) return hardenedSession;

  const embedded = session.fromPartition(EMBEDDED_PARTITION);

  embedded.setPermissionRequestHandler((_contents, permission, callback) => {
    log.info('[EmbeddedBrowser] denied permission request:', permission);
    callback(false);
  });
  embedded.setPermissionCheckHandler((_contents, permission) => {
    log.info('[EmbeddedBrowser] denied permission check:', permission);
    return false;
  });
  // WebUSB / Serial / HID.
  embedded.setDevicePermissionHandler(() => false);
  // `getDisplayMedia`. Returning an empty object denies without a prompt.
  embedded.setDisplayMediaRequestHandler((_request, callback) => callback({}));

  // The embedded page must never reach the daemon. It listens on loopback under
  // a per-launch secret, and a site that could talk to it would own the agent's
  // tools. `file:` is blocked for the same class of reason. Hostname requests
  // are routed through the pinned-IP proxy configured below; this hook covers
  // schemes that could otherwise avoid that network boundary.
  embedded.webRequest.onBeforeRequest({ urls: ['<all_urls>'] }, (details, callback) => {
    callback({ cancel: !isAllowedEmbeddedRequestUrl(details.url) });
  });

  // Downloads are the user's call, not the page's.
  embedded.on('will-download', (event, item) => {
    event.preventDefault();
    let origin = 'unknown origin';
    try {
      origin = new URL(item.getURL()).origin;
    } catch {
      // Keep signed URLs and query strings out of logs even when malformed.
    }
    log.info('[EmbeddedBrowser] blocked download from:', origin);
  });

  hardenedSession = embedded;
  void prepareEmbeddedNetwork(embedded);
  return embedded;
}

async function confirmPublicExternalNavigation(
  window: BrowserWindow,
  candidate: string,
  authentication: boolean,
  isCurrent: () => boolean
): Promise<boolean> {
  try {
    const validated = await validateExternalBrowserTarget(candidate);
    if (!isCurrent() || window.isDestroyed()) return false;
    const result = await dialog.showMessageBox(window, {
      type: 'question',
      buttons: ['Cancel', 'Open in browser'],
      defaultId: 0,
      cancelId: 0,
      noLink: true,
      message: authentication
        ? 'Open this sign-in page in your browser?'
        : 'Open this website in your browser?',
      detail: `Destination hostname: ${validated.hostname}\n\n${validated.href}`,
    });
    if (result.response !== 1 || !isCurrent()) return false;

    // The system browser performs its own DNS lookup, so an IP cannot be pinned
    // across this handoff. Revalidate, keep the confirmation tied to the exact
    // hostname, and require another prompt for every different destination.
    const revalidated = await validateExternalBrowserTarget(validated.href);
    if (
      revalidated.href !== validated.href ||
      revalidated.hostname !== validated.hostname ||
      !isCurrent()
    ) {
      return false;
    }
    await shell.openExternal(revalidated.href);
    return true;
  } catch {
    log.warn('[EmbeddedBrowser] refused external navigation');
    return false;
  }
}

async function confirmExternalAuthenticationNavigation(
  entry: Entry,
  viewId: string,
  candidate: string
): Promise<void> {
  await confirmPublicExternalNavigation(entry.window, candidate, true, () =>
    Boolean(entryFor(entry.window, viewId) === entry && !entry.window.isDestroyed())
  );
}

export async function openExternalBrowserNavigation(
  window: BrowserWindow,
  candidate: string
): Promise<boolean> {
  return confirmPublicExternalNavigation(
    window,
    candidate,
    isAuthenticationNavigation(candidate),
    () => !window.isDestroyed()
  );
}

function sourceRevision(entry: Entry): string {
  return `${entry.view.webContents.id}:${entry.revision}`;
}

function readState(entry: Entry, error: string | null = null): EmbeddedBrowserState {
  const contents = entry.view.webContents;
  return {
    url: contents.getURL().slice(0, MAX_PAGE_URL_CHARS),
    title: contents.getTitle().slice(0, MAX_PAGE_TITLE_CHARS),
    managedApp: Boolean(entry.managed),
    sourceRevision: sourceRevision(entry),
    canGoBack: contents.navigationHistory.canGoBack(),
    canGoForward: contents.navigationHistory.canGoForward(),
    isLoading: contents.isLoading(),
    error: error?.slice(0, MAX_PAGE_ERROR_CHARS) ?? null,
  };
}

export function createEmbeddedBrowser(
  window: BrowserWindow,
  viewId: string,
  initialUrl: string,
  onState: (state: EmbeddedBrowserState) => void,
  backend?: ManagedAppPreviewBackend,
  managedOnly = false
): EmbeddedBrowserState | null {
  if (window.isDestroyed() || !isNavigableEmbeddedUrl(initialUrl)) return null;
  const scope = managedAppPreviewScope(initialUrl, backend);
  if (managedOnly && !scope) return null;
  destroyEmbeddedBrowser(window, viewId);

  const managed = scope ? managedAppSession(window, scope) : undefined;
  const embedded = managed?.session ?? embeddedSession();
  const view = new WebContentsView({
    webPreferences: {
      session: embedded,
      // No preload at all. The bridge this app exposes is attached per
      // `webPreferences`; a view constructed without one has nothing to reach.
      // That is the structural guarantee, and it is why the toolbar is driven
      // from the main process rather than from inside the page.
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
      nodeIntegrationInWorker: false,
      nodeIntegrationInSubFrames: false,
      // An embedded page must not be able to spawn its own guest views.
      webviewTag: false,
      webSecurity: true,
      allowRunningInsecureContent: false,
      experimentalFeatures: false,
      plugins: false,
      safeDialogs: true,
      safeDialogsMessage: 'This page is showing too many dialogs.',
      navigateOnDragDrop: false,
      spellcheck: false,
    },
  });

  const contents = view.webContents;
  contents.setWebRTCIPHandlingPolicy('disable_non_proxied_udp');
  const bounds = { x: 0, y: 0, width: 0, height: 0 };
  let entry: Entry;
  let queuedAuthenticationUrl: string | null = null;
  let drainingAuthenticationQueue = false;
  const requestAuthenticationConfirmation = (url: string) => {
    queuedAuthenticationUrl = new URL(url).href;
    if (drainingAuthenticationQueue) return;
    drainingAuthenticationQueue = true;
    void (async () => {
      while (queuedAuthenticationUrl && entryFor(window, viewId) === entry) {
        const candidate = queuedAuthenticationUrl;
        queuedAuthenticationUrl = null;
        await confirmExternalAuthenticationNavigation(entry, viewId, candidate);
      }
    })().finally(() => {
      drainingAuthenticationQueue = false;
    });
  };
  entry = {
    view,
    window,
    visible: false,
    bounds,
    revision: 0,
    requestAuthenticationConfirmation,
    managed,
  };

  const push = (error: string | null = null) => {
    if (!contents.isDestroyed()) onState(readState(entry, error));
  };

  // Remote pages never get to create native dialogs. The toolbar's explicit
  // Open action remains the user-gesture path for leaving the embedded view.
  contents.setWindowOpenHandler(() => {
    log.info('[EmbeddedBrowser] blocked page-created window');
    push('This page tried to open another window. Use Open in browser if you want to continue.');
    return { action: 'deny' };
  });

  const guardNavigation = (event: Electron.Event, url: string) => {
    if (managed && (!managed.isActive() || !isManagedAppNavigation(managed.scope, url))) {
      event.preventDefault();
      push('This preview can only navigate within this app.');
      return;
    }
    if (!isNavigableEmbeddedUrl(url)) {
      log.warn('[EmbeddedBrowser] blocked navigation to a non-http(s) url');
      event.preventDefault();
      return;
    }
    if (!managed && isAuthenticationNavigation(url)) {
      event.preventDefault();
      push('Sign-in navigation was blocked. Use Open in browser to continue securely.');
    }
  };
  contents.on('will-navigate', guardNavigation);
  contents.on('will-redirect', guardNavigation);

  contents.on('did-start-loading', () => push());
  if (managed) {
    contents.on('dom-ready', () => {
      if (!contents.isDestroyed())
        void contents.executeJavaScript(PREVIEW_ACTIVITY_INSTALL).catch(() => {});
    });
  }
  contents.on('did-stop-loading', () => push());
  contents.on('did-navigate', () => {
    entry.revision += 1;
    push();
  });
  // Without this the URL bar goes stale on every single-page-app route change.
  contents.on('did-navigate-in-page', () => {
    entry.revision += 1;
    push();
  });
  contents.on('page-title-updated', () => push());
  contents.on('did-fail-load', (_event, errorCode, errorDescription, _url, isMainFrame) => {
    // Subframe failures are constant background noise, and -3 (ERR_ABORTED) is
    // what an ordinary user-cancelled navigation looks like.
    if (!isMainFrame || errorCode === -3) return;
    push(errorDescription || 'This page could not be loaded.');
  });
  contents.on('render-process-gone', () => push('This page stopped responding and was closed.'));

  window.contentView.addChildView(view);
  // BrowserView was transparent; WebContentsView defaults to white, which flashes
  // against a dark panel before the first paint.
  view.setBackgroundColor('#00000000');
  view.setBounds(bounds);
  view.setVisible(false);

  views.set(viewKey(window, viewId), entry);
  if (managed) {
    entry.releaseManagedView = managed.onRevoke(() => {
      push('The app backend stopped. Reopen the app after the backend restarts.');
      destroyEmbeddedBrowser(window, viewId);
    });
  }
  const networkReady = managed?.ready ?? prepareEmbeddedNetwork(embedded);
  void networkReady.then(
    () => {
      if (entryFor(window, viewId) === entry && !contents.isDestroyed()) {
        if (!managed && isAuthenticationNavigation(initialUrl)) {
          entry.requestAuthenticationConfirmation(initialUrl);
        } else if (managed) {
          // An app's page is served only to a browser holding its access cookie
          // (W2-HRD-1), so the preview opens the launch link that sets it.
          void managedAppLaunchUrl(managed.scope).then(
            (url) => {
              if (entryFor(window, viewId) === entry && !contents.isDestroyed()) {
                void contents.loadURL(url);
              }
            },
            (error: unknown) => {
              if (entryFor(window, viewId) === entry && !contents.isDestroyed()) {
                push(error instanceof Error ? error.message : 'This app could not be opened.');
              }
            }
          );
        } else {
          void contents.loadURL(initialUrl);
        }
      }
    },
    () => {
      if (entryFor(window, viewId) === entry && !contents.isDestroyed()) {
        push('The secure browser network could not be started.');
      }
    }
  );
  return readState(entry);
}

export function setEmbeddedBrowserBounds(
  window: BrowserWindow,
  viewId: string,
  bounds: EmbeddedBrowserBounds
): void {
  const entry = entryFor(window, viewId);
  if (!entry) return;
  entry.bounds = bounds;
  entry.view.setBounds({
    x: Math.round(bounds.x),
    y: Math.round(bounds.y),
    width: Math.max(0, Math.round(bounds.width)),
    height: Math.max(0, Math.round(bounds.height)),
  });
}

/**
 * Show or hide the view.
 *
 * The renderer calls this whenever something has to sit on top — a modal, a
 * dropdown, a toast, the resize shield — because a native view has no shared
 * z-index with the DOM and would otherwise paint straight over them.
 */
export function setEmbeddedBrowserVisible(
  window: BrowserWindow,
  viewId: string,
  visible: boolean
): void {
  const entry = entryFor(window, viewId);
  if (!entry) return;
  entry.visible = visible;
  entry.view.setVisible(visible);
}

export function navigateEmbeddedBrowser(
  window: BrowserWindow,
  viewId: string,
  url: string
): boolean {
  const entry = entryFor(window, viewId);
  if (!entry || !isNavigableEmbeddedUrl(url)) return false;
  if (
    entry.managed &&
    (!entry.managed.isActive() || !isManagedAppNavigation(entry.managed.scope, url))
  ) {
    return false;
  }
  if (!entry.managed && isAuthenticationNavigation(url)) {
    entry.requestAuthenticationConfirmation(url);
    return true;
  }
  void entry.view.webContents.loadURL(url);
  return true;
}

export function controlEmbeddedBrowser(
  window: BrowserWindow,
  viewId: string,
  action: 'back' | 'forward' | 'reload' | 'stop' | 'reload-if-idle'
): boolean | Promise<boolean> {
  const entry = entryFor(window, viewId);
  if (!entry || entry.view.webContents.isDestroyed()) return false;
  if (action === 'reload-if-idle') return reloadManagedAppIfIdle(window, viewId, entry);
  const contents = entry.view.webContents;
  if (action === 'back' && contents.navigationHistory.canGoBack()) {
    contents.navigationHistory.goBack();
  } else if (action === 'forward' && contents.navigationHistory.canGoForward()) {
    contents.navigationHistory.goForward();
  } else if (action === 'reload') {
    contents.reload();
  } else if (action === 'stop') {
    contents.stop();
  }
  return true;
}

async function reloadManagedAppIfIdle(
  window: BrowserWindow,
  viewId: string,
  entry: Entry
): Promise<boolean> {
  const contents = entry.view.webContents;
  if (!entry.managed || !entry.managed.isActive() || !entry.visible || contents.isLoading())
    return false;
  const revision = sourceRevision(entry);
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    // Only a boolean crosses back. Never inspect input values or accept script
    // from the renderer. A focused nested frame is conservatively considered busy.
    const idle = await Promise.race([
      contents.executeJavaScript(PREVIEW_ACTIVITY_IDLE),
      new Promise<boolean>((resolve) => {
        timer = setTimeout(() => resolve(false), 1000);
      }),
    ]);
    if (
      idle !== true ||
      window.isDestroyed() ||
      entryFor(window, viewId) !== entry ||
      contents.isDestroyed() ||
      contents.isLoading() ||
      !entry.visible ||
      sourceRevision(entry) !== revision ||
      !entry.managed.isActive() ||
      !isManagedAppNavigation(entry.managed.scope, contents.getURL())
    )
      return false;
    contents.reload();
    return true;
  } catch {
    return false;
  } finally {
    clearTimeout(timer);
  }
}

export function embeddedBrowserState(
  window: BrowserWindow,
  viewId: string
): EmbeddedBrowserState | null {
  const entry = entryFor(window, viewId);
  return entry ? readState(entry) : null;
}

/**
 * The visible text of the embedded page, for the agent.
 *
 * Runs in the page's own world via `executeJavaScript`, which is the only way
 * in: the view shares no origin with the renderer and has no preload. The
 * expression interpolates only a main-process-clamped integer limit, and the
 * result remains untrusted page content.
 */
export async function readEmbeddedBrowserText(
  window: BrowserWindow,
  viewId: string,
  maxChars: number
): Promise<{
  url: string;
  title: string;
  sourceRevision: string;
  text: string;
  truncated: boolean;
} | null> {
  const entry = entryFor(window, viewId);
  if (!entry) return null;
  const contents = entry.view.webContents;
  const limit = Number.isFinite(maxChars) ? Math.min(40_000, Math.max(0, Math.floor(maxChars))) : 0;
  // Never return the previous document while a navigation is visibly in
  // progress. A caller can retry once the toolbar reports the committed state.
  if (contents.isLoadingMainFrame()) return null;

  // A navigation can commit while executeJavaScript is crossing the process
  // boundary. Retry once so text, URL and revision always describe one document.
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const revision = entry.revision;
    const url = contents.getURL();
    const snapshot = (await contents.executeJavaScript(
      `(() => { const text = (document.body && document.body.innerText) || ""; const limit = ${limit}; return { text: text.slice(0, limit), truncated: text.length > limit }; })()`
    )) as { text?: unknown; truncated?: unknown };
    if (revision !== entry.revision || url !== contents.getURL()) continue;
    return {
      url: url.slice(0, MAX_PAGE_URL_CHARS),
      title: contents.getTitle().slice(0, MAX_PAGE_TITLE_CHARS),
      sourceRevision: sourceRevision(entry),
      text: typeof snapshot?.text === 'string' ? snapshot.text : '',
      truncated: snapshot?.truncated === true,
    };
  }
  return null;
}

/**
 * A PNG of the embedded page.
 *
 * `capturePage` grabs the compositor's current surface rather than forcing a
 * render, so it works while the view is occluded or off-screen, but returns the
 * *last painted frame* for a hidden window and an **empty image** if the view
 * was hidden and then navigated. Callers must check for the empty case — the
 * failure mode is a zero-byte buffer, not a rejection.
 */
export async function captureEmbeddedBrowser(
  window: BrowserWindow,
  viewId: string
): Promise<{ png: Buffer; width: number; height: number; sourceRevision: string } | null> {
  const entry = entryFor(window, viewId);
  if (!entry) return null;
  const contents = entry.view.webContents;
  if (contents.isLoadingMainFrame()) return null;
  const revision = sourceRevision(entry);
  const url = contents.getURL();
  const image = await contents.capturePage();
  if (image.isEmpty()) return null;
  if (revision !== sourceRevision(entry) || url !== contents.getURL()) return null;
  const size = image.getSize();
  return { png: image.toPNG(), width: size.width, height: size.height, sourceRevision: revision };
}

/** Everything a page stores, except cookies: see `clearManagedAppSiteData`. */
const MANAGED_APP_SITE_STORAGES: NonNullable<Electron.ClearStorageDataOptions['storages']> = [
  'filesystem',
  'indexdb',
  'localstorage',
  'shadercache',
  'websql',
  'serviceworkers',
  'cachestorage',
];

/**
 * "Clear site data" in a managed app preview.
 *
 * Everything the page stored goes, but not the app's access cookie (W2-HRD-1):
 * the daemon set it when the preview opened the app's one-time launch link, and
 * it is the preview's credential rather than the page's data. Clearing it made
 * the reload that follows answer "Open this app from Biorouter" until the
 * preview was closed and opened again, and that link cannot be opened a second
 * time. The access cookie is told apart by `HttpOnly`, which only a response can
 * set, so every cookie the page's own script wrote is still removed. The
 * session is this app's alone, so everything in it is this app's.
 */
async function clearManagedAppSiteData(target: Session, origin: string): Promise<void> {
  await target.clearStorageData({ origin, storages: MANAGED_APP_SITE_STORAGES });
  const cookies = await target.cookies.get({});
  await Promise.all(
    cookies
      .filter((cookie) => !cookie.httpOnly)
      .map((cookie) => target.cookies.remove(`${origin}${cookie.path || '/'}`, cookie.name))
  );
}

export async function clearEmbeddedBrowserData(
  window: BrowserWindow,
  viewId: string,
  allOrigins = false
): Promise<boolean> {
  const entry = entryFor(window, viewId);
  if (!entry) return false;
  const targetSession = entry.view.webContents.session;
  if (entry.managed) {
    await clearManagedAppSiteData(targetSession, entry.managed.scope.origin);
  } else if (allOrigins) {
    await targetSession.clearStorageData();
  } else {
    const origin = new URL(entry.view.webContents.getURL()).origin;
    await targetSession.clearStorageData({ origin });
  }
  return true;
}

export function destroyEmbeddedBrowser(window: BrowserWindow, viewId: string): void {
  const key = viewKey(window, viewId);
  const entry = views.get(key);
  if (!entry) return;
  views.delete(key);
  entry.releaseManagedView?.();
  try {
    entry.window.contentView.removeChildView(entry.view);
  } catch {
    // The window may already be gone; the view goes with it.
  }
  entry.view.webContents.close();
}

/** Tears down every view belonging to a window that is closing. */
export function destroyEmbeddedBrowsersForWindow(window: BrowserWindow): void {
  for (const [key, entry] of [...views.entries()]) {
    if (entry.window !== window) continue;
    views.delete(key);
    entry.releaseManagedView?.();
    try {
      window.contentView.removeChildView(entry.view);
    } catch {
      // The owner may already be tearing down.
    }
    entry.view.webContents.close();
  }
}

/** Reloads and renderer crashes do not run React cleanup; main owns this edge. */
export function registerEmbeddedBrowserOwnerTeardown(window: BrowserWindow): void {
  if (registeredOwnerTeardown.has(window)) return;
  registeredOwnerTeardown.add(window);
  window.webContents.on('did-start-navigation', (_event, _url, isInPlace, isMainFrame) => {
    if (isMainFrame && !isInPlace) destroyEmbeddedBrowsersForWindow(window);
  });
  window.webContents.on('render-process-gone', () => destroyEmbeddedBrowsersForWindow(window));
  window.webContents.on('destroyed', () => destroyEmbeddedBrowsersForWindow(window));
}
