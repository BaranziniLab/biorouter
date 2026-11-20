import {
  app,
  BrowserWindow,
  dialog,
  WebContentsView,
  session,
  shell,
  type Session,
  type WebContents,
  type WebContentsViewConstructorOptions,
} from 'electron';
import log from 'electron-log';
import path from 'node:path';
import {
  isAllowedEmbeddedRequestUrl,
  isAuthenticationNavigation,
  isPdfViewerResourceUrl,
} from './embeddedBrowserPolicy';
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
import {
  canonicalHost,
  decideLocalRequest,
  defaultPort,
  isReservedLocalPort,
  literalHostClass,
  resolveHostClass,
  wasRecentlyRefused,
} from '../browser/localNetwork';
import { acceptLanguages, chromeUserAgent } from '../browser/userAgent';
import {
  completedDownloadPath,
  handleAgentDownload,
  handlePersonDownload,
} from '../browser/downloads';
import { createBrowserStore, type BrowserStore } from '../browser/store';
import type {
  EmbeddedBrowserBookmark,
  EmbeddedBrowserDownload,
  EmbeddedBrowserFind,
  EmbeddedBrowserNotice,
  EmbeddedBrowserShortcut,
  EmbeddedBrowserState,
  EmbeddedBrowserSuggestion,
} from '../browser/types';

export type { EmbeddedBrowserState } from '../browser/types';

/**
 * The Biorouter Browser: a real, interactive browser inside the artifact panel
 * (`docs/desktop-ui/built-in-browser.md`, decisions BB-01 to BB-08).
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
 *
 * **One registry of tabs, owned here (BB-01).** A tab is a main process object keyed by its
 * window and a view id. The panel's live page is a viewport onto one: `create` attaches to the
 * tab when it exists, `release` hides it when the panel shows something else, and only `destroy`
 * (the panel tab closed, the panel unmounted, the window gone) ends it. Switching panel tabs used
 * to destroy the page and lose its history; now a tab keeps its page, its scroll position and its
 * back and forward list. A page can open a tab before any component exists (`target=_blank`), so
 * main creates it and asks the renderer to show it.
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
 * an embedded page reach the daemon's session. The name is kept from the first
 * version so that sites people signed in to stay signed in (BB-01).
 */
const EMBEDDED_PARTITION = 'persist:biorouter-embedded-browser';

export type EmbeddedBrowserBounds = { x: number; y: number; width: number; height: number };

/**
 * What the main process knows about one web contents of the browser partition: a tab or a
 * popup window. The local network rule (BB-06) and the popup rule (BB-04) read it.
 */
type ContentsRecord = {
  /** A main frame load the person started (address field, chat link, toolbar) is in flight. */
  personNavigation: boolean;
  /** The host of the committed page when it was admitted as a local page. */
  localPageHost: string | null;
  /** `host|port` keys admitted as local for main frame loads since the last commit. */
  admittedLocal: Set<string>;
  /** When the person last pressed a key, clicked or touched this page. */
  lastUserInputAt: number;
  /** The tab whose bar reports for this contents (a popup reports to its opener's tab). */
  entry: Entry | null;
};

type Entry = {
  view: WebContentsView;
  window: BrowserWindow;
  viewId: string;
  visible: boolean;
  bounds: EmbeddedBrowserBounds;
  revision: number;
  onState: (state: EmbeddedBrowserState) => void;
  managed?: ManagedAppPreviewSession;
  releaseManagedView?: () => void;
  record: ContentsRecord;
  error: string | null;
  errorOffersExternal: boolean;
  notice: EmbeddedBrowserNotice | null;
  download: EmbeddedBrowserDownload | null;
  find: EmbeddedBrowserFind | null;
  /** A tab a page opened waits this long for the panel to claim it, then closes. */
  claimTimer: ReturnType<typeof setTimeout> | null;
  noticedPermissions: Set<string>;
  /**
   * BB-15: the chat whose agent opened this tab, or opened the tab that opened it. `null` for a
   * tab the person opened, which the agent cannot see.
   */
  agentSession: string | null;
  /** The tab's id in the agent's tool results (`t2`). */
  agentLabel: string | null;
  /** While an agent operation dispatches input, input events are the agent's, not the person's. */
  agentInputDepth: number;
  /** When the person (not the agent) last pressed a key, clicked or touched this page. */
  personInputAt: number;
  /** BB-05: how a download this tab starts during an agent operation is handled. */
  agentDownload: AgentDownloadRoute | null;
};

/**
 * Set by the agent driver for the length of one operation on an agent tab (BB-05): a download
 * that starts then is the agent's. `dir` is set only under the act grant; with only the observe
 * grant the download is cancelled and `onDone` says why.
 */
export type AgentDownloadRoute = {
  dir: string | null;
  onDone: (download: AgentDownloadReport) => void;
};

export type AgentDownloadReport = {
  state: 'completed' | 'cancelled' | 'interrupted' | 'progressing';
  filename: string;
  bytes: number;
  mime: string;
  origin: string;
  path?: string;
  reason?: string;
};

const views = new Map<string, Entry>();
const records = new Map<number, ContentsRecord>();
const MAX_PAGE_TITLE_CHARS = 512;
const MAX_PAGE_URL_CHARS = 8 * 1024;
const MAX_PAGE_ERROR_CHARS = 1024;
/** Chromium's transient user activation lasts five seconds; a popup needs one (BB-04). */
const USER_GESTURE_WINDOW_MS = 5_000;
const CLAIM_TIMEOUT_MS = 30_000;
const ZOOM_FACTORS = [0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3];
let hardenedSession: Session | null = null;
let embeddedNetworkReady: Promise<void> | null = null;
let proxyTeardownRegistered = false;
let pageTabSequence = 0;
let store: BrowserStore | null = null;
const registeredOwnerTeardown = new WeakSet<BrowserWindow>();

function viewKey(window: BrowserWindow, viewId: string): string {
  return `${window.webContents.id}:${viewId}`;
}

function entryFor(window: BrowserWindow, viewId: string): Entry | undefined {
  return views.get(viewKey(window, viewId));
}

function browserStore(): BrowserStore {
  if (!store) {
    let dir: string | null = null;
    try {
      dir = path.join(app.getPath('userData'), 'biorouter-browser');
    } catch {
      dir = null;
    }
    // Without a user data folder (a unit test's stand in for Electron) the store lives in memory.
    store = createBrowserStore(dir ?? '', { persist: dir !== null });
    app.once('before-quit', () => store?.flush());
  }
  return store;
}

function newRecord(entry: Entry | null, inherit?: ContentsRecord | null): ContentsRecord {
  return {
    personNavigation: false,
    localPageHost: inherit?.localPageHost ?? null,
    admittedLocal: new Set(),
    lastUserInputAt: 0,
    entry,
  };
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

/** What a denied permission is called in the note under the bar. */
const PERMISSION_WORDS: Record<string, string> = {
  geolocation: 'your location',
  media: 'your camera or microphone',
  notifications: 'to show notifications',
  midi: 'MIDI devices',
  midiSysex: 'MIDI devices',
  'display-capture': 'to capture your screen',
  'window-management': 'to manage your windows',
  'idle-detection': 'to know when you are idle',
  'storage-access': 'access to its storage in other sites',
  'top-level-storage-access': 'access to its storage in other sites',
  'speaker-selection': 'your speakers',
  hid: 'a device',
  serial: 'a device',
  usb: 'a device',
};

function noticeDeniedPermission(contents: WebContents, permission: string, externalUrl?: string) {
  const entry = records.get(contents.id)?.entry;
  if (!entry || entry.managed) return;
  if (permission === 'openExternal') {
    // A link to another app (`mailto:`, `zoommtg:`). The browser never launches apps.
    let scheme = '';
    try {
      if (externalUrl) scheme = new URL(externalUrl).protocol.replace(/:$/, '');
    } catch {
      scheme = '';
    }
    setNotice(entry, {
      kind: 'external-app',
      text: scheme
        ? `This page tried to open a ${scheme} link in another app. The Biorouter Browser does not open other apps.`
        : 'This page tried to open another app. The Biorouter Browser does not open other apps.',
    });
    return;
  }
  const words = PERMISSION_WORDS[permission];
  if (!words || entry.noticedPermissions.has(permission)) return;
  entry.noticedPermissions.add(permission);
  setNotice(entry, {
    kind: 'permission',
    text: `This site asked for ${words}. The Biorouter Browser does not grant it.`,
    action: { label: 'Open in default browser', kind: 'open-external', url: currentUrl(entry) },
  });
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
 * `navigator.permissions.query` and device enumeration. The one permission
 * granted is `clipboard-sanitized-write`, so a page's Copy button works (BB-08).
 */
function embeddedSession(): Session {
  if (hardenedSession) return hardenedSession;

  const embedded = session.fromPartition(EMBEDDED_PARTITION);

  // BB-07: the plain Chrome user agent for the bundled Chromium, never `Electron/…`.
  embedded.setUserAgent(
    chromeUserAgent(process.platform, process.versions.chrome ?? ''),
    acceptLanguages(app.getPreferredSystemLanguages?.() ?? [])
  );

  embedded.setPermissionRequestHandler((contents, permission, callback, details) => {
    if (permission === 'clipboard-sanitized-write') {
      callback(true);
      return;
    }
    log.info('[EmbeddedBrowser] denied permission request:', permission);
    callback(false);
    if (contents) {
      noticeDeniedPermission(
        contents,
        permission,
        (details as { externalURL?: string } | undefined)?.externalURL
      );
    }
  });
  embedded.setPermissionCheckHandler((_contents, permission) => {
    if (permission === 'clipboard-sanitized-write') return true;
    // ⚠ No note from a check: Chromium checks media, geolocation and window management on
    // every page load (measured on Electron 39.8.10), so a check is not a site asking. A denied
    // `getCurrentPosition` is answered from the check alone and so gets no note either.
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
  // schemes that could otherwise avoid that network boundary, and decides which
  // local destinations a request may reach (BB-06, `browser/localNetwork.ts`).
  embedded.webRequest.onBeforeRequest({ urls: ['<all_urls>'] }, (details, callback) => {
    if (isPdfViewerResourceUrl(details.url)) {
      callback({});
      return;
    }
    if (!isAllowedEmbeddedRequestUrl(details.url)) {
      callback({ cancel: true });
      return;
    }
    let url: URL;
    try {
      url = new URL(details.url);
    } catch {
      callback({ cancel: true });
      return;
    }
    const contentsId = details.webContents?.id;
    const record = contentsId === undefined ? undefined : records.get(contentsId);
    const mainFrame = details.resourceType === 'mainFrame';
    let requesterHost: string | null = null;
    if (!mainFrame && details.resourceType !== 'subFrame') {
      try {
        const frameUrl = details.frame?.url;
        if (frameUrl && /^(https?|wss?):/.test(frameUrl)) {
          requesterHost = canonicalHost(new URL(frameUrl).hostname);
        }
      } catch {
        requesterHost = null;
      }
    }
    void decideLocalRequest(
      {
        url,
        person: Boolean(mainFrame && record?.personNavigation),
        localPageHost: record?.localPageHost ?? null,
        requesterHost,
      },
      resolveHostClass
    ).then(
      (verdict) => {
        if (verdict.allow) {
          if (mainFrame && record && verdict.cls && verdict.cls !== 'public') {
            record.admittedLocal.add(`${canonicalHost(url.hostname)}|${defaultPort(url)}`);
          }
          callback({});
          return;
        }
        log.info('[EmbeddedBrowser] refused a local destination:', verdict.reason);
        if (mainFrame && record?.entry) noticeLocalRefusal(record.entry, url, verdict.reason);
        callback({ cancel: true });
      },
      () => callback({ cancel: true })
    );
  });

  // BB-05: a download the person starts goes through the save dialog. One no tab owns (a
  // service worker, a page already gone) has nobody to ask, so it is cancelled.
  embedded.on('will-download', (event, item, contents) => {
    const entry = contents ? records.get(contents.id)?.entry : undefined;
    if (!entry || entry.window.isDestroyed()) {
      event.preventDefault();
      log.info('[EmbeddedBrowser] cancelled a download no tab owns');
      return;
    }
    if (entry.agentDownload) {
      handleAgentDownload(item, entry.agentDownload.dir, entry.agentDownload.onDone);
      return;
    }
    handlePersonDownload(item, (state) => {
      entry.download = state;
      pushState(entry);
    });
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

/**
 * The page's address as it is shown outside the view. A managed app's launch
 * token is left out: the preview briefly commits the daemon's answer to the
 * launch link (`/apps/<id>/?t=…`) before it moves on to the app, and although
 * the token is spent by then, it has no business in the toolbar or in what the
 * agent is shown.
 */
function shownUrl(entry: Entry, url: string): string {
  if (!entry.managed) return url;
  try {
    const parsed = new URL(url);
    if (!parsed.searchParams.has('t')) return url;
    parsed.searchParams.delete('t');
    return parsed.href;
  } catch {
    return url;
  }
}

function currentUrl(entry: Entry): string {
  return entry.view.webContents.isDestroyed() ? '' : entry.view.webContents.getURL();
}

function readState(entry: Entry): EmbeddedBrowserState {
  const contents = entry.view.webContents;
  const url = contents.getURL();
  return {
    url: shownUrl(entry, url).slice(0, MAX_PAGE_URL_CHARS),
    title: contents.getTitle().slice(0, MAX_PAGE_TITLE_CHARS),
    managedApp: Boolean(entry.managed),
    sourceRevision: sourceRevision(entry),
    canGoBack: contents.navigationHistory.canGoBack(),
    canGoForward: contents.navigationHistory.canGoForward(),
    isLoading: contents.isLoading(),
    error: entry.error?.slice(0, MAX_PAGE_ERROR_CHARS) ?? null,
    errorOffersExternal: entry.errorOffersExternal,
    notice: entry.notice,
    download: entry.download,
    isLocal: entry.record.localPageHost !== null,
    zoomFactor: entry.managed ? 1 : contents.getZoomFactor(),
    bookmarked: entry.managed ? false : browserStore().isBookmarked(url),
    find: entry.find,
  };
}

function pushState(entry: Entry): void {
  if (!entry.view.webContents.isDestroyed()) entry.onState(readState(entry));
}

function setNotice(entry: Entry, notice: EmbeddedBrowserNotice | null): void {
  entry.notice = notice;
  pushState(entry);
}

function noticeLocalRefusal(entry: Entry, url: URL, reason: 'reserved' | 'local'): void {
  const where = url.port ? `${url.hostname}:${url.port}` : url.hostname;
  if (reason === 'reserved') {
    setNotice(entry, {
      kind: 'local-reserved',
      text: `${where} belongs to Biorouter itself, so the browser does not open it.`,
    });
    return;
  }
  setNotice(entry, {
    kind: 'local-blocked',
    text: `This page tried to open a local address, ${where}. Pages can't open local addresses on their own.`,
    action: { label: 'Open it', kind: 'open-here', url: url.href },
  });
}

function sendToOwner(entry: Entry, channel: string, payload: unknown): void {
  const owner = entry.window.webContents;
  if (!entry.window.isDestroyed() && !owner.isDestroyed()) owner.send(channel, payload);
}

function sendShortcut(entry: Entry, action: EmbeddedBrowserShortcut['action']): void {
  if (action !== 'close-tab' && !entry.window.isDestroyed()) {
    // The address field and the find field live in the renderer, which must hold focus first.
    entry.window.webContents.focus();
  }
  sendToOwner(entry, 'embedded-browser:shortcut', { viewId: entry.viewId, action });
}

const HARDENED_PREFERENCES = {
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
  // The built in PDF viewer is the only plugin there is. Without it a linked paper
  // downloads instead of opening (measured: with it, a PDF renders in the tab).
  plugins: true,
  safeDialogs: true,
  safeDialogsMessage: 'This page is showing too many dialogs.',
  navigateOnDragDrop: false,
  spellcheck: false,
} as const;

function stepZoom(contents: WebContents, direction: 'in' | 'out' | 'reset'): number {
  if (direction === 'reset') {
    contents.setZoomFactor(1);
    return 1;
  }
  const current = contents.getZoomFactor();
  const next =
    direction === 'in'
      ? (ZOOM_FACTORS.find((factor) => factor > current + 0.001) ??
        ZOOM_FACTORS[ZOOM_FACTORS.length - 1])
      : ([...ZOOM_FACTORS].reverse().find((factor) => factor < current - 0.001) ?? ZOOM_FACTORS[0]);
  contents.setZoomFactor(next);
  return next;
}

/** Load a page as the person's own navigation (BB-06 admits local addresses for these). */
function loadAsPerson(entry: Entry, url: string): void {
  entry.record.personNavigation = true;
  entry.error = null;
  entry.errorOffersExternal = false;
  entry.notice = null;
  void entry.view.webContents.loadURL(url).catch(() => {
    // Failures arrive through did-fail-load, which sets the error the bar shows.
  });
}

function isPageTabDisposition(disposition: string): boolean {
  return disposition !== 'new-window';
}

function popupSize(features: string): { width: number; height: number } {
  const read = (name: string, fallback: number) => {
    const match = new RegExp(`(?:^|,)\\s*${name}\\s*=\\s*(\\d+)`, 'i').exec(features);
    const value = match ? Number(match[1]) : fallback;
    return Math.min(1400, Math.max(320, value));
  };
  return { width: read('width', 600), height: read('height', 700) };
}

/**
 * New windows and tabs a page opens (BB-04).
 *
 * - `target=_blank`, a middle click, `window.open` without features: a new browser tab in the
 *   same panel. Main creates the view through `createWindow`, so `window.opener` and the
 *   request's referrer survive, and asks the renderer to show it.
 * - A sized `window.open` (an OAuth or payment popup): a small child window of the app window on
 *   the same partition, without preload, closed by the page's own `window.close()`.
 * - Anything without a user gesture in the last five seconds: blocked, with a note offering to
 *   open it in a new tab.
 *
 * Every target must be an http(s) URL or `about:blank` (a popup a page fills itself), so
 * `file:`, `data:`, `javascript:` and credentialed URLs are still refused.
 */
function installWindowOpenHandler(
  contents: WebContents,
  record: ContentsRecord,
  reportTo: () => Entry | null
): void {
  contents.setWindowOpenHandler((details) => {
    const entry = reportTo();
    const target = details.url;
    const navigable = target === 'about:blank' || isNavigableEmbeddedUrl(target);
    if (!entry || entry.window.isDestroyed() || !navigable) {
      log.info('[EmbeddedBrowser] blocked a page-created window');
      return { action: 'deny' };
    }
    if (entry.managed) {
      // A managed app preview never leaves its app. A link out of it can still be
      // opened, by the person, as an ordinary browser tab.
      if (isNavigableEmbeddedUrl(target) && !isManagedAppNavigation(entry.managed.scope, target)) {
        setNotice(entry, {
          kind: 'popup-blocked',
          text: 'This app tried to open a website in another window.',
          action: { label: 'Open in a new tab', kind: 'open-tab', url: target },
        });
      }
      return { action: 'deny' };
    }
    const gesture = Date.now() - record.lastUserInputAt <= USER_GESTURE_WINDOW_MS;
    // Opening a window spends the gesture, as `window.open` spends user activation.
    record.lastUserInputAt = 0;
    if (!gesture) {
      log.info('[EmbeddedBrowser] blocked a window opened without a user gesture');
      if (isNavigableEmbeddedUrl(target)) {
        setNotice(entry, {
          kind: 'popup-blocked',
          text: 'This page tried to open a new window.',
          action: { label: 'Open in a new tab', kind: 'open-tab', url: target },
        });
      }
      return { action: 'deny' };
    }

    if (isPageTabDisposition(details.disposition)) {
      return {
        action: 'allow',
        outlivesOpener: true,
        createWindow: (options) => {
          const view = new WebContentsView(options as WebContentsViewConstructorOptions);
          pageTabSequence += 1;
          const viewId = `page-tab-${pageTabSequence}`;
          const child = registerEntry(entry.window, viewId, view, {
            onState: (state) =>
              sendToOwnerByWindow(entry.window, 'embedded-browser:state', { viewId, state }),
            inherit: record,
          });
          // A tab an agent tab opened is the agent's too, and the agent sees it (BB-04).
          if (entry.agentSession) {
            child.agentSession = entry.agentSession;
            child.agentLabel = nextAgentLabel(entry.window, entry.agentSession);
          }
          child.claimTimer = setTimeout(() => {
            child.claimTimer = null;
            log.info('[EmbeddedBrowser] closed a page-opened tab nobody showed');
            destroyEmbeddedBrowser(entry.window, viewId);
          }, CLAIM_TIMEOUT_MS);
          sendToOwner(entry, 'embedded-browser:opened', {
            openerViewId: entry.viewId,
            viewId,
            url: target,
            foreground: details.disposition !== 'background-tab',
          });
          return view.webContents;
        },
      };
    }

    const size = popupSize(details.features);
    return {
      action: 'allow',
      overrideBrowserWindowOptions: {
        parent: entry.window,
        width: size.width,
        height: size.height,
        autoHideMenuBar: true,
        title: 'Biorouter Browser',
        webPreferences: { ...HARDENED_PREFERENCES, session: embeddedSession() },
      },
    };
  });

  contents.on('did-create-window', (child) => {
    hardenPopupWindow(child, record, reportTo);
  });
}

function sendToOwnerByWindow(window: BrowserWindow, channel: string, payload: unknown): void {
  if (!window.isDestroyed() && !window.webContents.isDestroyed()) {
    window.webContents.send(channel, payload);
  }
}

/** Shared by tabs and popups: the guards that make a page a page and nothing more. */
function installContentsGuards(contents: WebContents, record: ContentsRecord): void {
  records.set(contents.id, record);
  contents.once('destroyed', () => {
    if (records.get(contents.id) === record) records.delete(contents.id);
  });
  contents.setWebRTCIPHandlingPolicy('disable_non_proxied_udp');
  // BB-04: a page may open a window only right after the person acted on it.
  contents.on('input-event', (_event, input) => {
    if (
      input.type === 'mouseDown' ||
      input.type === 'keyDown' ||
      input.type === 'rawKeyDown' ||
      input.type === 'touchStart' ||
      input.type === 'gestureTapDown'
    ) {
      record.lastUserInputAt = Date.now();
      // The person's own input, told apart from the agent's: the next agent result says the
      // person interacted with the page (BB-13).
      if (record.entry && record.entry.agentInputDepth === 0)
        record.entry.personInputAt = Date.now();
    }
  });
  // BB-08: never pick a client certificate for a site (Electron's default is the first one).
  contents.on('select-client-certificate', (event, _url, _list, callback) => {
    event.preventDefault();
    (callback as (certificate?: Electron.Certificate) => void)();
    if (record.entry) {
      setNotice(record.entry, {
        kind: 'client-certificate',
        text: 'This site asked for a client certificate, which the Biorouter Browser does not send.',
        action: {
          label: 'Open in default browser',
          kind: 'open-external',
          url: currentUrl(record.entry),
        },
      });
    }
  });
  // BB-08: HTTP authentication is cancelled in this version, and the bar says so.
  contents.on('login', (event, _details, _authInfo, callback) => {
    event.preventDefault();
    (callback as (username?: string, password?: string) => void)();
    if (record.entry) {
      setNotice(record.entry, {
        kind: 'auth',
        text: 'This site asked for a username and password, which the Biorouter Browser does not support yet.',
        action: {
          label: 'Open in default browser',
          kind: 'open-external',
          url: currentUrl(record.entry),
        },
      });
    }
  });
}

/** A popup window a page opened (BB-04): the same partition and guards, no preload. */
function hardenPopupWindow(
  popup: BrowserWindow,
  openerRecord: ContentsRecord,
  reportTo: () => Entry | null
): void {
  const contents = popup.webContents;
  const record = newRecord(openerRecord.entry, openerRecord);
  installContentsGuards(contents, record);
  installWindowOpenHandler(contents, record, reportTo);
  const guard = (event: Electron.Event, url: string) => {
    if (url !== 'about:blank' && !isNavigableEmbeddedUrl(url)) {
      event.preventDefault();
      return;
    }
    record.personNavigation = false;
  };
  contents.on('will-navigate', guard);
  contents.on('will-redirect', (event, url) => {
    if (!isNavigableEmbeddedUrl(url)) event.preventDefault();
  });
  contents.on('did-navigate', (_event, url) => commitRecord(record, url));
  contents.on('before-input-event', (event, input) => {
    if (input.type !== 'keyDown') return;
    const mod = process.platform === 'darwin' ? input.meta : input.control;
    // The app menu's Close Tab sends to the focused window's renderer, and a popup has none.
    if (mod && !input.alt && !input.shift && input.key.toLowerCase() === 'w') {
      event.preventDefault();
      popup.close();
    }
  });
  popup.setMenuBarVisibility(false);
}

function commitRecord(record: ContentsRecord, url: string): void {
  record.personNavigation = false;
  // A new document has no user activation, as in Chromium: a click on the page that was left
  // does not let the next one open a window.
  record.lastUserInputAt = 0;
  try {
    const parsed = new URL(url);
    const host = canonicalHost(parsed.hostname);
    const key = `${host}|${defaultPort(parsed)}`;
    record.localPageHost = record.admittedLocal.has(key) ? host : null;
  } catch {
    record.localPageHost = null;
  }
  record.admittedLocal.clear();
}

function registerEntry(
  window: BrowserWindow,
  viewId: string,
  view: WebContentsView,
  options: {
    onState: (state: EmbeddedBrowserState) => void;
    managed?: ManagedAppPreviewSession;
    inherit?: ContentsRecord | null;
  }
): Entry {
  const contents = view.webContents;
  const bounds = { x: 0, y: 0, width: 0, height: 0 };
  const entry: Entry = {
    view,
    window,
    viewId,
    visible: false,
    bounds,
    revision: 0,
    onState: options.onState,
    managed: options.managed,
    record: newRecord(null, options.inherit),
    error: null,
    errorOffersExternal: false,
    notice: null,
    download: null,
    find: null,
    claimTimer: null,
    noticedPermissions: new Set(),
    agentSession: null,
    agentLabel: null,
    agentInputDepth: 0,
    personInputAt: 0,
    agentDownload: null,
  };
  entry.record.entry = entry;
  const { managed } = options;

  if (!managed) {
    installContentsGuards(contents, entry.record);
    installWindowOpenHandler(contents, entry.record, () =>
      entryFor(window, viewId) === entry ? entry : null
    );
  } else {
    records.set(contents.id, entry.record);
    contents.once('destroyed', () => {
      if (records.get(contents.id) === entry.record) records.delete(contents.id);
    });
    contents.setWebRTCIPHandlingPolicy('disable_non_proxied_udp');
    installWindowOpenHandler(contents, entry.record, () =>
      entryFor(window, viewId) === entry ? entry : null
    );
  }

  const guardNavigation = (event: Electron.Event, url: string) => {
    if (managed && (!managed.isActive() || !isManagedAppNavigation(managed.scope, url))) {
      event.preventDefault();
      entry.error = 'This preview can only navigate within this app.';
      pushState(entry);
      return;
    }
    if (!isNavigableEmbeddedUrl(url)) {
      log.warn('[EmbeddedBrowser] blocked navigation to a non-http(s) url');
      event.preventDefault();
      return;
    }
  };
  contents.on('will-navigate', (event, url) => {
    // A navigation the page itself started (a link, a form, a script) is not the person's
    // address bar, so it never carries the person's local access.
    entry.record.personNavigation = false;
    guardNavigation(event, url);
  });
  contents.on('will-redirect', guardNavigation);

  contents.on('did-start-navigation', (details) => {
    if (details.isMainFrame && !details.isSameDocument) {
      entry.error = null;
      entry.errorOffersExternal = false;
      // A note about the page being left goes with it; one about this navigation is set
      // after this event, by the request filter or the failure handler.
      entry.notice = null;
      entry.noticedPermissions.clear();
      entry.find = null;
    }
  });
  contents.on('did-start-loading', () => pushState(entry));
  if (managed) {
    contents.on('dom-ready', () => {
      if (!contents.isDestroyed())
        void contents.executeJavaScript(PREVIEW_ACTIVITY_INSTALL).catch(() => {});
    });
  }
  contents.on('did-stop-loading', () => {
    entry.record.personNavigation = false;
    pushState(entry);
  });
  contents.on('did-navigate', (_event, url) => {
    entry.revision += 1;
    commitRecord(entry.record, url);
    // The person's own history only: pages the agent opened are not suggestions for the person.
    if (!managed && !entry.agentSession) browserStore().recordVisit(url, contents.getTitle());
    pushState(entry);
  });
  // Without this the URL bar goes stale on every single-page-app route change.
  contents.on('did-navigate-in-page', (_event, url, isMainFrame) => {
    entry.revision += 1;
    if (isMainFrame) entry.record.personNavigation = false;
    if (!managed && isMainFrame && !entry.agentSession) {
      browserStore().recordVisit(url, contents.getTitle());
    }
    pushState(entry);
  });
  contents.on('page-title-updated', (_event, title) => {
    if (!managed) browserStore().updateTitle(contents.getURL(), title);
    pushState(entry);
  });
  contents.on('did-fail-load', (_event, errorCode, errorDescription, validatedUrl, isMainFrame) => {
    // Subframe failures are constant background noise, and -3 (ERR_ABORTED) is
    // what an ordinary user-cancelled navigation looks like.
    if (!isMainFrame || errorCode === -3) return;
    entry.record.personNavigation = false;
    if (entry.notice?.kind === 'local-blocked' || entry.notice?.kind === 'local-reserved') {
      pushState(entry);
      return;
    }
    let failed: URL | null = null;
    try {
      failed = new URL(validatedUrl);
    } catch {
      failed = null;
    }
    if (failed && wasRecentlyRefused(canonicalHost(failed.hostname), defaultPort(failed))) {
      const reserved =
        isReservedLocalPort(defaultPort(failed)) &&
        literalHostClass(canonicalHost(failed.hostname)) === 'loopback';
      noticeLocalRefusal(entry, failed, reserved ? 'reserved' : 'local');
      return;
    }
    // Chromium's certificate errors are -200 to -299 (net/base/net_error_list.h).
    if (errorCode <= -200 && errorCode > -300) {
      entry.error = `The connection to ${failed?.hostname ?? 'this site'} is not secure (${errorDescription || 'certificate error'}), so the page was not loaded.`;
      entry.errorOffersExternal = true;
    } else {
      entry.error = errorDescription || 'This page could not be loaded.';
      entry.errorOffersExternal = false;
    }
    pushState(entry);
  });
  contents.on('render-process-gone', () => {
    entry.error = 'This page stopped responding and was closed.';
    pushState(entry);
  });
  contents.on('found-in-page', (_event, result) => {
    entry.find = { activeMatchOrdinal: result.activeMatchOrdinal, matches: result.matches };
    pushState(entry);
  });
  contents.on('zoom-changed', (_event, direction) => {
    if (managed) return;
    stepZoom(contents, direction === 'in' ? 'in' : 'out');
    pushState(entry);
  });

  // Keys pressed while the page has focus reach the page, not the app, so the
  // browser's own keys are mapped here. `before-input-event` runs before the
  // application menu, so these also win over its accelerators.
  contents.on('before-input-event', (event, input) => {
    if (input.type !== 'keyDown') return;
    const mod = process.platform === 'darwin' ? input.meta : input.control;
    const key = input.key.toLowerCase();
    const take = () => event.preventDefault();
    if (mod && !input.alt && key === 'l') {
      take();
      sendShortcut(entry, 'focus-address');
    } else if (mod && !input.alt && key === 'r') {
      take();
      entry.record.personNavigation = true;
      if (input.shift) contents.reloadIgnoringCache();
      else contents.reload();
    } else if (mod && !input.alt && (key === '[' || key === ']')) {
      take();
      entry.record.personNavigation = true;
      if (key === '[' && contents.navigationHistory.canGoBack())
        contents.navigationHistory.goBack();
      if (key === ']' && contents.navigationHistory.canGoForward())
        contents.navigationHistory.goForward();
    } else if (
      process.platform !== 'darwin' &&
      input.alt &&
      !input.control &&
      (key === 'arrowleft' || key === 'arrowright')
    ) {
      take();
      entry.record.personNavigation = true;
      if (key === 'arrowleft' && contents.navigationHistory.canGoBack())
        contents.navigationHistory.goBack();
      if (key === 'arrowright' && contents.navigationHistory.canGoForward())
        contents.navigationHistory.goForward();
    } else if (mod && !input.alt && !input.shift && key === 'f' && !managed) {
      take();
      sendShortcut(entry, 'find');
    } else if (mod && !input.alt && !input.shift && key === 'w') {
      take();
      sendShortcut(entry, 'close-tab');
    } else if (mod && !input.alt && (key === '=' || key === '+') && !managed) {
      take();
      stepZoom(contents, 'in');
      pushState(entry);
    } else if (mod && !input.alt && key === '-' && !managed) {
      take();
      stepZoom(contents, 'out');
      pushState(entry);
    } else if (mod && !input.alt && key === '0' && !managed) {
      take();
      stepZoom(contents, 'reset');
      pushState(entry);
    } else if (!mod && !input.alt && key === 'escape' && contents.isLoading()) {
      take();
      contents.stop();
    }
  });

  window.contentView.addChildView(view);
  // BrowserView was transparent; WebContentsView defaults to white, which flashes
  // against a dark panel before the first paint.
  view.setBackgroundColor('#00000000');
  if (!managed) {
    // ...but once a page has painted, a page that sets no background of its own must sit on
    // white, as in Chrome: transparent, it showed the panel through it, which in dark mode put
    // black text on a dark ground (measured with the fixture site, whose pages set no colour).
    contents.once('dom-ready', () => {
      if (!contents.isDestroyed()) view.setBackgroundColor('#ffffff');
    });
  }
  view.setBounds(bounds);
  view.setVisible(false);
  views.set(viewKey(window, viewId), entry);
  return entry;
}

/**
 * Show the browser tab `viewId` in `window`, creating it at `initialUrl` when it does not exist
 * yet. A tab that exists (the panel showed it before, or a page opened it) is attached as it is:
 * its page, history and scroll position stay, and `initialUrl` is ignored.
 */
export function createEmbeddedBrowser(
  window: BrowserWindow,
  viewId: string,
  initialUrl: string,
  onState: (state: EmbeddedBrowserState) => void,
  backend?: ManagedAppPreviewBackend,
  managedOnly = false
): EmbeddedBrowserState | null {
  if (window.isDestroyed()) return null;
  const existing = entryFor(window, viewId);
  if (existing && !existing.view.webContents.isDestroyed()) {
    const scope = managedAppPreviewScope(initialUrl, backend);
    const sameKind = existing.managed
      ? Boolean(
          scope && existing.managed.isActive() && existing.managed.scope.appId === scope.appId
        )
      : !managedOnly;
    if (sameKind) {
      existing.onState = onState;
      if (existing.claimTimer) {
        clearTimeout(existing.claimTimer);
        existing.claimTimer = null;
      }
      return readState(existing);
    }
  }
  if (!isNavigableEmbeddedUrl(initialUrl)) return null;
  const scope = managedAppPreviewScope(initialUrl, backend);
  if (managedOnly && !scope) return null;
  destroyEmbeddedBrowser(window, viewId);

  const managed = scope ? managedAppSession(window, scope) : undefined;
  const embedded = managed?.session ?? embeddedSession();
  const view = new WebContentsView({
    webPreferences: {
      session: embedded,
      ...HARDENED_PREFERENCES,
    },
  });
  const contents = view.webContents;
  const entry = registerEntry(window, viewId, view, { onState, managed });

  if (managed) {
    entry.releaseManagedView = managed.onRevoke(() => {
      entry.error = 'The app backend stopped. Reopen the app after the backend restarts.';
      pushState(entry);
      destroyEmbeddedBrowser(window, viewId);
    });
  }
  // The person opened this page (a chat link, Open here, the address field), so its first
  // load is theirs (BB-06).
  if (!managed) entry.record.personNavigation = true;
  const networkReady = managed?.ready ?? prepareEmbeddedNetwork(embedded);
  void networkReady.then(
    () => {
      if (entryFor(window, viewId) === entry && !contents.isDestroyed()) {
        if (managed) {
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
                entry.error =
                  error instanceof Error ? error.message : 'This app could not be opened.';
                pushState(entry);
              }
            }
          );
        } else {
          loadAsPerson(entry, initialUrl);
        }
      }
    },
    () => {
      if (entryFor(window, viewId) === entry && !contents.isDestroyed()) {
        entry.error = 'The secure browser network could not be started.';
        pushState(entry);
      }
    }
  );
  return readState(entry);
}

/**
 * The panel showed something else: hide the tab and keep it. Only `destroy` ends a tab.
 */
export function releaseEmbeddedBrowser(window: BrowserWindow, viewId: string): void {
  const entry = entryFor(window, viewId);
  if (!entry) return;
  entry.visible = false;
  entry.view.setVisible(false);
}

/** The panel took a tab a page opened; it stays until the panel closes it. */
export function claimEmbeddedBrowser(window: BrowserWindow, viewId: string): boolean {
  const entry = entryFor(window, viewId);
  if (!entry) return false;
  if (entry.claimTimer) {
    clearTimeout(entry.claimTimer);
    entry.claimTimer = null;
  }
  return true;
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

/** The person navigates: the address field, a chat link clicked again, a note's Open it. */
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
  if (entry.managed) {
    void entry.view.webContents.loadURL(url);
    return true;
  }
  loadAsPerson(entry, url);
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
    entry.record.personNavigation = true;
    contents.navigationHistory.goBack();
  } else if (action === 'forward' && contents.navigationHistory.canGoForward()) {
    entry.record.personNavigation = true;
    contents.navigationHistory.goForward();
  } else if (action === 'reload') {
    entry.record.personNavigation = true;
    contents.reload();
  } else if (action === 'stop') {
    contents.stop();
  }
  return true;
}

/** The browser bar's ⋯ menu and keys: zoom, find, the note, bookmarks. */
export function zoomEmbeddedBrowser(
  window: BrowserWindow,
  viewId: string,
  direction: 'in' | 'out' | 'reset'
): number | null {
  const entry = entryFor(window, viewId);
  if (!entry || entry.managed || entry.view.webContents.isDestroyed()) return null;
  const factor = stepZoom(entry.view.webContents, direction);
  pushState(entry);
  return factor;
}

export function findInEmbeddedBrowser(
  window: BrowserWindow,
  viewId: string,
  text: string,
  options: { forward: boolean; findNext: boolean }
): boolean {
  const entry = entryFor(window, viewId);
  if (!entry || entry.managed || entry.view.webContents.isDestroyed()) return false;
  const query = text.slice(0, 1000);
  if (!query) {
    entry.view.webContents.stopFindInPage('clearSelection');
    entry.find = null;
    pushState(entry);
    return true;
  }
  entry.view.webContents.findInPage(query, {
    forward: options.forward,
    findNext: options.findNext,
  });
  return true;
}

export function stopFindInEmbeddedBrowser(window: BrowserWindow, viewId: string): void {
  const entry = entryFor(window, viewId);
  if (!entry || entry.view.webContents.isDestroyed()) return;
  entry.view.webContents.stopFindInPage('keepSelection');
  entry.find = null;
  pushState(entry);
}

export function dismissEmbeddedBrowserNotice(window: BrowserWindow, viewId: string): void {
  const entry = entryFor(window, viewId);
  if (!entry) return;
  entry.notice = null;
  if (entry.download && entry.download.state !== 'progressing') entry.download = null;
  pushState(entry);
}

export function showEmbeddedBrowserDownload(window: BrowserWindow, viewId: string): boolean {
  const entry = entryFor(window, viewId);
  const id = entry?.download?.id;
  const saved = id ? completedDownloadPath(id) : null;
  if (!saved) return false;
  shell.showItemInFolder(saved);
  return true;
}

export function setEmbeddedBrowserBookmark(
  window: BrowserWindow,
  viewId: string,
  bookmarked: boolean
): boolean {
  const entry = entryFor(window, viewId);
  if (!entry || entry.managed || entry.view.webContents.isDestroyed()) return false;
  const contents = entry.view.webContents;
  const changed = bookmarked
    ? browserStore().addBookmark(contents.getURL(), contents.getTitle())
    : browserStore().removeBookmark(contents.getURL());
  pushState(entry);
  return changed;
}

export function embeddedBrowserBookmarks(): EmbeddedBrowserBookmark[] {
  return browserStore()
    .bookmarks()
    .map((bookmark) => ({ url: bookmark.url, title: bookmark.title }));
}

export function embeddedBrowserSuggestions(query: string): EmbeddedBrowserSuggestion[] {
  if (typeof query !== 'string' || query.length > 512) return [];
  return browserStore().suggest(query, 8);
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
      url: shownUrl(entry, url).slice(0, MAX_PAGE_URL_CHARS),
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

function closeEntry(entry: Entry): void {
  if (entry.claimTimer) {
    clearTimeout(entry.claimTimer);
    entry.claimTimer = null;
  }
  entry.releaseManagedView?.();
  try {
    entry.window.contentView.removeChildView(entry.view);
  } catch {
    // The window may already be gone; the view goes with it.
  }
  entry.view.webContents.close();
}

export function destroyEmbeddedBrowser(window: BrowserWindow, viewId: string): void {
  const key = viewKey(window, viewId);
  const entry = views.get(key);
  if (!entry) return;
  views.delete(key);
  closeEntry(entry);
}

/** Tears down every view belonging to a window that is closing. */
export function destroyEmbeddedBrowsersForWindow(window: BrowserWindow): void {
  for (const [key, entry] of [...views.entries()]) {
    if (entry.window !== window) continue;
    views.delete(key);
    closeEntry(entry);
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

// ---------------------------------------------------------------------------------------------
// Agent tabs (BB-12 to BB-15). The driver in `browser/agent/driver.ts` uses these; nothing else
// in main gives the agent a way to a page. An agent tab is an ordinary browser tab of the
// window, on the same partition and with the same guards, that remembers which chat's agent
// opened it. The person's own tabs have no `agentSession` and stay invisible to the agent.
// ---------------------------------------------------------------------------------------------

const agentLabelCounters = new Map<string, number>();
let agentTabSequence = 0;

function agentKey(window: BrowserWindow, sessionId: string): string {
  return `${window.webContents.id}:${sessionId}`;
}

function nextAgentLabel(window: BrowserWindow, sessionId: string): string {
  const key = agentKey(window, sessionId);
  const next = (agentLabelCounters.get(key) ?? 0) + 1;
  agentLabelCounters.set(key, next);
  return `t${next}`;
}

export type AgentTabInfo = {
  viewId: string;
  label: string;
  contents: WebContents;
  visible: boolean;
  url: string;
  title: string;
  isLoading: boolean;
  personInputAt: number;
};

function agentTabInfo(entry: Entry): AgentTabInfo {
  const contents = entry.view.webContents;
  return {
    viewId: entry.viewId,
    label: entry.agentLabel ?? entry.viewId,
    contents,
    visible: isShown(entry),
    url: contents.isDestroyed() ? '' : contents.getURL().slice(0, MAX_PAGE_URL_CHARS),
    title: contents.isDestroyed() ? '' : contents.getTitle().slice(0, MAX_PAGE_TITLE_CHARS),
    isLoading: !contents.isDestroyed() && contents.isLoading(),
    personInputAt: entry.personInputAt,
  };
}

function isShown(entry: Entry): boolean {
  return (
    entry.visible &&
    entry.bounds.width > 0 &&
    entry.bounds.height > 0 &&
    !entry.window.isDestroyed() &&
    !entry.window.isMinimized()
  );
}

/** The agent tabs of one chat in one window, oldest first. */
export function agentBrowserTabs(window: BrowserWindow, sessionId: string): AgentTabInfo[] {
  return [...views.values()]
    .filter(
      (entry) =>
        entry.window === window &&
        entry.agentSession === sessionId &&
        !entry.view.webContents.isDestroyed()
    )
    .map(agentTabInfo);
}

export function agentBrowserTab(window: BrowserWindow, viewId: string): AgentTabInfo | null {
  const entry = entryFor(window, viewId);
  if (!entry || !entry.agentSession || entry.view.webContents.isDestroyed()) return null;
  return agentTabInfo(entry);
}

/** A new, empty agent tab for this chat. The driver loads it once the panel shows it. */
export function createAgentBrowserTab(
  window: BrowserWindow,
  sessionId: string
): AgentTabInfo | null {
  if (window.isDestroyed()) return null;
  agentTabSequence += 1;
  const viewId = `agent-tab-${agentTabSequence}`;
  const view = new WebContentsView({
    webPreferences: { session: embeddedSession(), ...HARDENED_PREFERENCES },
  });
  const entry = registerEntry(window, viewId, view, {
    onState: (state) => sendToOwnerByWindow(window, 'embedded-browser:state', { viewId, state }),
  });
  entry.agentSession = sessionId;
  entry.agentLabel = nextAgentLabel(window, sessionId);
  // A renderer for the debugger to attach to: a web contents that never loaded has none, and
  // CDP commands to it wait for one.
  void view.webContents.loadURL('about:blank').catch(() => undefined);
  return agentTabInfo(entry);
}

/** Ask the chat's panel to show an agent tab (BB-15: the agent drives only what is visible). */
export function showAgentBrowserTab(window: BrowserWindow, viewId: string): void {
  const entry = entryFor(window, viewId);
  if (!entry?.agentSession) return;
  const contents = entry.view.webContents;
  sendToOwnerByWindow(window, 'biorouter-browser:show-tab', {
    sessionId: entry.agentSession,
    viewId,
    url: contents.isDestroyed() ? '' : contents.getURL(),
    title: contents.isDestroyed() ? '' : contents.getTitle(),
  });
}

/** The agent stopped using a tab it opened: the panel closes its tab, and the view ends. */
export function closeAgentBrowserTab(window: BrowserWindow, viewId: string): boolean {
  const entry = entryFor(window, viewId);
  if (!entry?.agentSession) return false;
  sendToOwnerByWindow(window, 'biorouter-browser:close-tab', {
    sessionId: entry.agentSession,
    viewId,
  });
  destroyEmbeddedBrowser(window, viewId);
  return true;
}

/** An agent operation started or ended on a tab: the bar shows it, with Stop (BB-15). */
export function reportAgentActivity(window: BrowserWindow, viewId: string, active: boolean): void {
  const entry = entryFor(window, viewId);
  if (!entry?.agentSession) return;
  sendToOwnerByWindow(window, 'biorouter-browser:activity', {
    sessionId: entry.agentSession,
    viewId,
    active,
  });
}

/**
 * Navigate an agent tab (BB-06 and BB-20). A public chat's agent never opens a private network
 * or loopback address; a private chat's agent may, with the person's own admission rule. The
 * daemon's port and the other reserved ports stay refused for everyone (the request filter).
 */
export async function navigateAgentBrowserTab(
  window: BrowserWindow,
  viewId: string,
  url: string,
  tier: 'public' | 'private'
): Promise<{ ok: true } | { ok: false; reason: string }> {
  const entry = entryFor(window, viewId);
  if (!entry?.agentSession || entry.view.webContents.isDestroyed()) {
    return { ok: false, reason: 'The tab is gone.' };
  }
  if (!isNavigableEmbeddedUrl(url)) {
    return {
      ok: false,
      reason: 'Only http and https addresses without credentials can be opened.',
    };
  }
  const parsed = new URL(url);
  const host = canonicalHost(parsed.hostname);
  const cls =
    literalHostClass(host) ?? (await resolveHostClass(host).catch(() => null))?.cls ?? null;
  if (cls === null) {
    return { ok: false, reason: `${parsed.hostname} could not be found.` };
  }
  if (cls !== 'public' && tier !== 'private') {
    return {
      ok: false,
      reason: `${parsed.hostname} is a local or private network address, which a chat on a public model may not open (privacy tiers). The person can open it, or use a private model.`,
    };
  }
  if (isReservedLocalPort(defaultPort(parsed)) && cls === 'loopback') {
    return {
      ok: false,
      reason: `${parsed.host} belongs to Biorouter itself, so the browser does not open it.`,
    };
  }
  await prepareEmbeddedNetwork(entry.view.webContents.session);
  if (entryFor(window, viewId) !== entry || entry.view.webContents.isDestroyed()) {
    return { ok: false, reason: 'The tab closed.' };
  }
  entry.record.personNavigation = cls !== 'public' && tier === 'private';
  entry.error = null;
  entry.errorOffersExternal = false;
  entry.notice = null;
  void entry.view.webContents.loadURL(url).catch(() => {
    // A failed load arrives through did-fail-load, which the driver reads from the state.
  });
  return { ok: true };
}

/** Back, forward, reload, stop on an agent tab: page navigation, never the person's. */
export function controlAgentBrowserTab(
  window: BrowserWindow,
  viewId: string,
  action: 'back' | 'forward' | 'reload' | 'stop'
): boolean {
  const entry = entryFor(window, viewId);
  if (!entry?.agentSession || entry.view.webContents.isDestroyed()) return false;
  const history = entry.view.webContents.navigationHistory;
  entry.record.personNavigation = false;
  if (action === 'back') {
    if (!history.canGoBack()) return false;
    history.goBack();
  } else if (action === 'forward') {
    if (!history.canGoForward()) return false;
    history.goForward();
  } else if (action === 'reload') {
    entry.view.webContents.reload();
  } else {
    entry.view.webContents.stop();
  }
  return true;
}

/** The last load error the bar shows for a tab (a refused local address, a certificate error). */
export function agentBrowserTabError(window: BrowserWindow, viewId: string): string | null {
  const entry = entryFor(window, viewId);
  if (!entry) return null;
  return entry.error ?? (entry.notice ? entry.notice.text : null);
}

/**
 * Bracket the agent's own input on a tab: input events inside are the agent's (so they never
 * read as the person interacting), and a click inside counts as the gesture a page needs to
 * open a tab (BB-04), because a popup the agent's click opens is a tab the agent then sees.
 */
export function beginAgentInput(window: BrowserWindow, viewId: string): void {
  const entry = entryFor(window, viewId);
  if (!entry) return;
  entry.agentInputDepth += 1;
  entry.record.lastUserInputAt = Date.now();
}

export function endAgentInput(window: BrowserWindow, viewId: string): void {
  const entry = entryFor(window, viewId);
  if (!entry) return;
  // Input events arrive asynchronously; keep the agent's attribution a moment longer.
  setTimeout(() => {
    entry.agentInputDepth = Math.max(0, entry.agentInputDepth - 1);
  }, 150);
}

export function setAgentDownloadRoute(
  window: BrowserWindow,
  viewId: string,
  route: AgentDownloadRoute | null
): void {
  const entry = entryFor(window, viewId);
  if (entry) entry.agentDownload = route;
}

const FOCUSED_EDITABLE_EXPRESSION = `(() => {
  const active = document.activeElement;
  if (!active) return false;
  if (active instanceof HTMLInputElement) return !active.disabled && !active.readOnly;
  if (active instanceof HTMLTextAreaElement) return !active.disabled && !active.readOnly;
  return active instanceof HTMLElement && active.isContentEditable;
})()`;

/**
 * The isolated world the **Dictate into this app** focus check runs in (S15). World 0 is the
 * page's own main world, where the app's script can replace `document.activeElement`, the
 * element constructors or `instanceof` itself and so decide the answer; 999 is the world
 * Electron gives a preload under context isolation. This id is used for nothing else, so the
 * check sees the shared DOM through globals the page cannot reach.
 */
export const DICTATION_FOCUS_WORLD_ID = 1907;

export async function managedAppHasFocusedField(
  window: BrowserWindow,
  viewId: string
): Promise<boolean> {
  const entry = entryFor(window, viewId);
  if (!entry?.managed?.isActive() || entry.view.webContents.isDestroyed()) return false;
  try {
    return (
      (await entry.view.webContents.executeJavaScriptInIsolatedWorld(DICTATION_FOCUS_WORLD_ID, [
        { code: FOCUSED_EDITABLE_EXPRESSION },
      ])) === true
    );
  } catch {
    return false;
  }
}

export async function insertIntoManagedAppFocusedField(
  window: BrowserWindow,
  viewId: string,
  text: string
): Promise<boolean> {
  if (!text || text.length > 32_000 || !(await managedAppHasFocusedField(window, viewId)))
    return false;
  const entry = entryFor(window, viewId);
  if (!entry?.managed?.isActive() || entry.view.webContents.isDestroyed()) return false;
  entry.view.webContents.focus();
  entry.view.webContents.insertText(text);
  return true;
}
