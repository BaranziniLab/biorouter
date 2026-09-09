import Electron, { contextBridge, ipcRenderer, webUtils } from 'electron';
import { Workflow } from './workflow';

// One-time warning for callers still using the legacy `off()` API. Each
// channel only warns once to avoid log spam under React StrictMode.
const offDeprecationWarned = new Set<string>();
function warnOffDeprecated(channel: string): void {
  if (offDeprecationWarned.has(channel)) return;
  offDeprecationWarned.add(channel);
  console.warn(
    `[preload] window.electron.off('${channel}', ...) is a no-op. ` +
      'Use the disposer returned by on(): `const dispose = electron.on(...); return dispose;`.'
  );
}

interface NotificationData {
  title: string;
  body: string;
}

interface MessageBoxOptions {
  type?: 'none' | 'info' | 'error' | 'question' | 'warning';
  buttons?: string[];
  defaultId?: number;
  title?: string;
  message: string;
  detail?: string;
}

interface MessageBoxResponse {
  response: number;
  checkboxChecked?: boolean;
}

interface SaveDialogOptions {
  title?: string;
  defaultPath?: string;
  buttonLabel?: string;
  filters?: Array<{ name: string; extensions: string[] }>;
  message?: string;
  nameFieldLabel?: string;
  showsTagField?: boolean;
}

interface SaveDialogResponse {
  canceled: boolean;
  filePath?: string;
}

interface SaveDiagnosticsResponse extends SaveDialogResponse {
  error?: string;
}

interface FileResponse {
  file: string;
  filePath: string;
  error: string | null;
  found: boolean;
}

interface SaveDataUrlResponse {
  id: string;
  filePath?: string;
  error?: string;
}

// Mirrors `utils/embeddedBrowser.ts`. Declared here rather than imported because
// preload is bundled separately and must not pull in main-process modules.
type EmbeddedBrowserState = {
  url: string;
  title: string;
  managedApp?: boolean;
  sourceRevision: string;
  canGoBack: boolean;
  canGoForward: boolean;
  isLoading: boolean;
  error: string | null;
};

type ArtifactFileEntry = {
  name: string;
  path: string;
  relativePath: string;
  parentPath: string;
  isDirectory: boolean;
  size?: number;
};

type ArtifactDocumentFormat = 'pdf' | 'docx' | 'xlsx' | 'pptx';

type ArtifactGitStatus = 'untracked' | 'modified' | 'staged' | 'committed' | 'pushed';

type ArtifactGitEntry = {
  name: string;
  path: string;
  relativePath: string;
  parentPath: string;
  isDirectory: boolean;
  status: ArtifactGitStatus;
};

type ArtifactFilePreview =
  | {
      kind: 'text' | 'html';
      title: string;
      path: string;
      mimeType: string;
      text: string;
      size: number;
      revision?: string;
      found: true;
    }
  | {
      kind: 'image';
      title: string;
      path: string;
      mimeType: string;
      dataUrl?: string;
      bytes?: ArrayBuffer;
      size: number;
      revision?: string;
      found: true;
    }
  | {
      kind: 'document';
      format: ArtifactDocumentFormat;
      title: string;
      path: string;
      mimeType: string;
      data: ArrayBuffer;
      size: number;
      revision?: string;
      extractedText?: string;
      textTruncated?: boolean;
      found: true;
    }
  | {
      kind: 'directory';
      title: string;
      path: string;
      entries: ArtifactFileEntry[];
      found: true;
    }
  | {
      kind: 'gitDirectory';
      title: string;
      path: string;
      branch: string;
      entries: ArtifactGitEntry[];
      found: true;
    }
  | {
      kind: 'binary';
      title: string;
      path: string;
      mimeType: string;
      size: number;
      found: true;
    }
  | {
      kind: 'error';
      title: string;
      path: string;
      /** Friendly, human-readable message — never the raw Node errno string. */
      error: string;
      /**
       * Errno-style code ('ENOENT', 'EACCES', …) when one was recognised.
       * Mirrors `ArtifactFilePreview` in components/artifacts/artifactTypes.ts
       * — keep the two unions in lockstep so the renderer needs no casts.
       */
      code?: string;
      found: false;
    };

/**
 * One entry of a batched file-existence check.
 *
 * Mirrors `FilePathCheckRequest` in components/artifacts/fileLinkStatus.ts —
 * preload cannot import renderer modules, so the shape is restated here the way
 * `ArtifactFilePreview` above is.
 */
type FilePathCheckRequest = { path: string; workingDir?: string };
/** The whole answer to one such entry. */
type FilePathCheckResult = { exists: boolean; isDirectory: boolean };

type TerminalCreateResult =
  | {
      success: true;
      sessionId: string;
      cwd: string;
      backend: 'pty' | 'process';
    }
  | {
      success: false;
      error: string;
    };

type TerminalActionResult = { success: true } | { success: false; error: string };

type TerminalDataEvent = {
  sessionId: string;
  data: string;
};

type TerminalExitEvent = {
  sessionId: string;
  exitCode: number | null;
  signal: string | null;
};

const config = JSON.parse(process.argv.find((arg) => arg.startsWith('{')) || '{}');

interface UpdaterEvent {
  event: string;
  data?: unknown;
}

// Define the API types in a single place
type ElectronAPI = {
  platform: string;
  reactReady: () => void;
  getConfig: () => Record<string, unknown>;
  hideWindow: () => void;
  directoryChooser: () => Promise<Electron.OpenDialogReturnValue>;
  createChatWindow: (
    query?: string,
    dir?: string,
    version?: string,
    resumeSessionId?: string,
    viewType?: string,
    workflowId?: string
  ) => void;
  createDivergedChatWindow: (
    dir: string | undefined,
    resumeSessionId: string,
    resumeSessionTitle?: string
  ) => void;
  logInfo: (txt: string) => void;
  showNotification: (data: NotificationData) => void;
  showMessageBox: (options: MessageBoxOptions) => Promise<MessageBoxResponse>;
  showSaveDialog: (options: SaveDialogOptions) => Promise<SaveDialogResponse>;
  saveDiagnosticsBundle: (
    sessionId: string,
    archive: ArrayBuffer
  ) => Promise<SaveDiagnosticsResponse>;
  openInChrome: (url: string) => void;
  fetchMetadata: (url: string) => Promise<string>;
  reloadApp: () => void;
  checkForOllama: () => Promise<boolean>;
  selectFileOrDirectory: (defaultPath?: string) => Promise<string | null>;
  getBinaryPath: (binaryName: string) => Promise<string>;
  importSessionFile: () => Promise<string | null>;
  readFile: (directory: string) => Promise<FileResponse>;
  readArtifactFile: (filePath: string) => Promise<ArtifactFilePreview>;
  /**
   * Does each of these paths exist, and is it a directory? One answer per
   * request, in order.
   *
   * Batched deliberately: a chat message can name thirty paths, and the
   * renderer asks about all of them at once rather than thirty round trips.
   * The reply carries nothing but the two booleans — never file contents, never
   * a directory listing — so it cannot become a second, weaker read channel
   * beside `readArtifactFile`.
   */
  checkFilePaths: (requests: FilePathCheckRequest[]) => Promise<FilePathCheckResult[]>;
  writeFile: (directory: string, content: string) => Promise<boolean>;
  ensureDirectory: (dirPath: string) => Promise<boolean>;
  listFiles: (dirPath: string, extension?: string) => Promise<string[]>;
  listSkillDirs: (dirPath: string) => Promise<string[]>;
  deleteFile: (filePath: string) => Promise<boolean>;
  deleteDirectory: (dirPath: string) => Promise<boolean>;
  getAllowedExtensions: () => Promise<string[]>;
  getPathForFile: (file: File) => string;
  setMenuBarIcon: (show: boolean) => Promise<boolean>;
  getMenuBarIconState: () => Promise<boolean>;
  setDockIcon: (show: boolean) => Promise<boolean>;
  getDockIconState: () => Promise<boolean>;
  getSettings: () => Promise<unknown | null>;
  saveSettings: (settings: unknown) => Promise<boolean>;
  getSecretKey: () => Promise<string>;
  /**
   * Issue #56 DR-16: the proof a tier-raising request came from the user.
   * Attached as `X-User-Action` on those requests only — not as a default
   * header, which would make it ambient on every call.
   */
  getUserActionKey: () => Promise<string>;
  getBiorouterdHostPort: () => Promise<string | null>;
  setWakelock: (enable: boolean) => Promise<boolean>;
  getWakelockState: () => Promise<boolean>;
  setSpellcheck: (enable: boolean) => Promise<boolean>;
  getSpellcheckState: () => Promise<boolean>;
  openNotificationsSettings: () => Promise<boolean>;
  onMouseBackButtonClicked: (callback: () => void) => () => void;
  offMouseBackButtonClicked: (callback: () => void) => void;
  /** Subscribe to a main-process IPC event. Returns a disposer; call it to
   * remove the listener. Do not use `off()` — contextBridge proxies the
   * callback differently on each crossing, so `off(channel, sameCallback)`
   * can't find the registered wrapper and silently leaks the listener. */
  on: (
    channel: string,
    callback: (event: Electron.IpcRendererEvent, ...args: unknown[]) => void
  ) => () => void;
  /** Deprecated. Use the disposer returned by `on()` instead. Calling this
   * is a no-op (kept for source compatibility); the listener will leak. */
  off: (
    channel: string,
    callback: (event: Electron.IpcRendererEvent, ...args: unknown[]) => void
  ) => void;
  emit: (channel: string, ...args: unknown[]) => void;
  broadcastThemeChange: (themeData: {
    mode: string;
    useSystemTheme: boolean;
    theme: string;
    themeFamily?: string;
  }) => void;
  // Functions for image pasting
  saveDataUrlToTemp: (dataUrl: string, uniqueId: string) => Promise<SaveDataUrlResponse>;
  deleteTempFile: (filePath: string) => void;
  // Opens only after public-target validation and exact-host native confirmation.
  openExternal: (url: string) => Promise<void>;
  // Function to serve temp images
  getTempImage: (filePath: string) => Promise<string | null>;
  // Function to read temp image as raw base64 + mimeType for API use
  readTempImageAsBase64: (filePath: string) => Promise<{ data: string; mimeType: string }>;
  // Update-related functions
  getVersion: () => string;
  checkForUpdates: () => Promise<{ updateInfo: unknown; error: string | null }>;
  downloadUpdate: () => Promise<{ success: boolean; error: string | null }>;
  installUpdate: () => void;
  restartApp: () => void;
  /** Subscribe to main-process updater events. Returns a disposer that removes
   * the listener; call it on unmount to avoid duplicate registrations. */
  onUpdaterEvent: (callback: (event: UpdaterEvent) => void) => () => void;
  getUpdateState: () => Promise<{
    updateAvailable: boolean;
    latestVersion?: string;
    status?: 'checking' | 'available' | 'downloaded' | 'up-to-date' | 'error';
    percent?: number;
    usingFallback?: boolean;
    error?: string;
  } | null>;
  isUsingGitHubFallback: () => Promise<boolean>;
  // Workflow warning functions
  closeWindow: () => void;
  /**
   * The tab band's titlebar gestures. `windowDragStart` hands the window to a
   * cursor-following timer in main, and `windowDragEnd` is what stops it —
   * main cannot see the mouse button come up, so the renderer OWES it that
   * message from every path that can end a press. See
   * `titlebarWindowGesture.ts` and `useTabBandWindowGesture.ts`.
   */
  windowDragStart: () => void;
  windowDragEnd: () => void;
  windowToggleZoom: () => void;
  ensureWindowContentWidth: (width: number) => Promise<{
    expanded: boolean;
    width: number;
    height: number;
  }>;
  hasAcceptedWorkflowBefore: (workflow: Workflow) => Promise<boolean>;
  recordWorkflowHash: (workflow: Workflow) => Promise<boolean>;
  openDirectoryInExplorer: (directoryPath: string) => Promise<boolean>;
  captureRegion: (payload: {
    x: number;
    y: number;
    width: number;
    height: number;
    label?: string;
    containment?: { x: number; y: number; width: number; height: number };
  }) => Promise<{ path: string; width: number; height: number } | null>;
  embeddedBrowser: {
    isManagedAppUrl: (url: string) => Promise<boolean>;
    create: (
      viewId: string,
      url: string,
      managedOnly?: boolean
    ) => Promise<EmbeddedBrowserState | null>;
    setBounds: (
      viewId: string,
      bounds: { x: number; y: number; width: number; height: number }
    ) => Promise<void>;
    setVisible: (viewId: string, visible: boolean) => Promise<void>;
    navigate: (viewId: string, url: string) => Promise<boolean>;
    control: (
      viewId: string,
      action: 'back' | 'forward' | 'reload' | 'stop' | 'reload-if-idle'
    ) => Promise<boolean>;
    destroy: (viewId: string) => Promise<void>;
    readText: (
      viewId: string,
      maxChars: number
    ) => Promise<{
      url: string;
      title: string;
      sourceRevision: string;
      text: string;
      truncated: boolean;
    } | null>;
    capture: (viewId: string) => Promise<{
      path: string;
      width: number;
      height: number;
      sourceRevision: string;
    } | null>;
    clearData: (viewId: string) => Promise<boolean>;
    onState: (
      callback: (payload: { viewId: string; state: EmbeddedBrowserState }) => void
    ) => () => void;
  };
  openArtifactInBrowser: (payload: {
    html: string;
    title?: string;
    theme?: 'light' | 'dark';
  }) => Promise<{ ok: boolean }>;
  prepareArtifactHtml: (payload: { html: string }) => Promise<{ html: string }>;
  addRecentDir: (dir: string) => Promise<boolean>;
  openBrxtFilePicker: () => Promise<string | null>;
  validateBrxtBundle: (filePath: string) => Promise<
    | {
        manifest: import('./types/brxt').BrxtManifest;
        skillsPreview: Array<{ slug: string; name: string; description: string }>;
      }
    | { error: string }
  >;
  uninstallBrxtExtension: (extensionName: string) => Promise<{ success: true } | { error: string }>;
  /**
   * Issue #56 Task 43 (DR-23). `registrySource` carries the BAAM registry `id`
   * the bundle came from, so the install can record provenance beside the
   * config entry and the daemon can re-derive the privacy tier from it instead
   * of from the (locally renameable) config name. Omitted for a hand-dropped
   * `.brxt`, which has no registry id to record.
   */
  installBrxtBundle: (
    filePath: string,
    extensionName: string,
    registrySource?: { registryId: string; sourceUrl?: string }
  ) => Promise<{ success: true; installDir: string } | { error: string }>;
  // BAAM registry (Browse Skills / Browse Extensions)
  /**
   * Issue #56 §10.2. `stale: true` means the main process could not reach the
   * registry and answered from its last-good copy; `fetchedAt` dates whichever
   * document came back, so the renderer can say how old the catalogue is instead
   * of only that it is not live.
   */
  fetchRegistry: () => Promise<
    | {
        registry: import('./components/baam/registry').BaamRegistry;
        fetchedAt?: string;
        stale?: boolean;
      }
    | { error: string }
  >;
  downloadRegistryAsset: (url: string) => Promise<{ path: string } | { error: string }>;
  // Dependency checker
  checkDependencies: (opts?: {
    force?: boolean;
  }) => Promise<import('./utils/dependencyChecker').DependencyInfo[]>;
  installDependency: (dep: string) => Promise<{ started: boolean } | { error: string }>;
  /**
   * The environment as the MAIN process sees it — which is not the environment
   * the user's login shell sees. A debugging session that doesn't know that
   * spends its first turns rediscovering it.
   */
  dependencyEnvironment: () => Promise<
    import('./utils/dependencyDebugPrompt').DependencyEnvironment & {
      uvResolvesTo?: string | null;
    }
  >;
  // Biorouter CLI install (delegates to the bundled `biorouter setup-path`)
  cliStatus: () => Promise<{
    bundled: string | null;
    onPath: boolean;
    pathLocation: string | null;
    bundledVersion: string | null;
    pathVersion: string | null;
    needsUpdate: boolean;
    brokenOnPath: boolean;
  }>;
  installCli: () => Promise<
    { success: true; output: string } | { success: false; error: string; command?: string }
  >;
  launchCli: (
    workingDir?: string
  ) => Promise<{ success: true } | { success: false; error: string }>;
  createTerminalSession: (options?: {
    workingDir?: string;
    cols?: number;
    rows?: number;
  }) => Promise<TerminalCreateResult>;
  writeTerminalSession: (sessionId: string, data: string) => Promise<TerminalActionResult>;
  resizeTerminalSession: (
    sessionId: string,
    cols: number,
    rows: number
  ) => Promise<TerminalActionResult>;
  disposeTerminalSession: (sessionId: string) => Promise<TerminalActionResult>;
  onTerminalData: (callback: (event: TerminalDataEvent) => void) => () => void;
  onTerminalExit: (callback: (event: TerminalExitEvent) => void) => () => void;
  // Extension updater (events pushed via 'extension-update-event' channel).
  // Returns a disposer, like every other `on*` above — see the implementation.
  onExtensionUpdateEvent: (
    callback: (event: import('./utils/extensionUpdater').ExtensionUpdateEvent) => void
  ) => () => void;

  // ── Tab tear-off and merge (docs/design/astryx-adoption/tab-tear-off-and-merge.md)
  //
  // Four sends and one invoke. The RECEIVE direction needs nothing new: the
  // generic `on(channel, cb)` above already returns a disposer, and the two
  // inbound channels — 'tab-drag:preview' and 'tab-drag:merge' — carry plain
  // JSON.
  //
  // Why the source window is never told which window it is over: it does not
  // render anything differently. D7 gives `detach` and `merge` the SAME ghost
  // (flat, outlined, clamped to this window's frame) because the merge caret is
  // painted by the TARGET, not here. Sending a phase back would be an IPC
  // message per pointermove whose only consumer is a value nobody reads — and it
  // would hand this renderer another window's id, which D3 deliberately keeps
  // out of its reach.
  tabDragRegisterBands: (bands: TabDragBand[]) => void;
  tabDragMove: (point: TabDragScreenPoint) => void;
  tabDragCommit: (request: TabDragCommitRequest) => Promise<TabDragCommitResult>;
  /** Cancelled or returned inside: clear every window's preview. */
  tabDragEnd: () => void;
  /** Target → main: the insert this window was asked for is done (or refused). */
  tabDragAckMerge: (requestId: number, inserted: boolean) => void;
};

/** A tab-strip rectangle in the reporting window's VIEWPORT coordinates. */
type TabDragBand = { x: number; y: number; width: number; height: number };

/** Raw `PointerEvent.screenX/screenY`. Main normalises to DIP (design D4). */
type TabDragScreenPoint = { screenX: number; screenY: number };

/**
 * The serialisable subset of a tab — mirrors `TabMovePayload` in
 * `components/chatGroups/tabTearOff.ts`, which is where the exclusions (`tabId`,
 * the pending-message cargo) are justified. Restated structurally here rather
 * than imported: preload must not pull renderer modules into the isolated
 * world, and this type is a wire format, not shared logic.
 */
type TabDragTab = {
  sessionId: string;
  title: string;
  userSetName: boolean;
  cwd?: string;
  workflowId?: string;
};

type TabDragCommitRequest = {
  point: TabDragScreenPoint;
  /** Where inside the tab the pointer grabbed it, so the new window lands under the cursor. */
  grabOffset: { x: number; y: number };
  tab: TabDragTab;
  /**
   * This is the source window's LAST tab, across every group in its layout.
   *
   * It rides the request rather than being decided in the renderer because the
   * two outcomes disagree about it, and only main knows which one this is:
   * a **tear-off** of the last tab is a no-op (D5 — destroying a window to
   * rebuild an identical one costs a renderer boot and a per-session extension
   * reload), while a **merge** of the last tab is allowed and closes the source
   * window afterwards (D6a). The renderer cannot tell a tear-off from a merge;
   * it only knows the pointer left.
   */
  isOnlyTab: boolean;
};

/**
 * `merge` and `detach` both mean the source must now drop its tab. `noop` means
 * it must NOT — the merge target refused the insert, or the drop resolved to
 * nothing — and is the only answer that leaves the tab where it was.
 */
type TabDragCommitResult = { outcome: 'merge' | 'detach' | 'noop' };

type AppConfigAPI = {
  get: (key: string) => unknown;
  getAll: () => Record<string, unknown>;
};

const electronAPI: ElectronAPI = {
  platform: process.platform,
  reactReady: () => ipcRenderer.send('react-ready'),
  getConfig: () => {
    if (!config || Object.keys(config).length === 0) {
      console.warn(
        'No config provided by main process. This may indicate an initialization issue.'
      );
    }
    return config;
  },
  hideWindow: () => ipcRenderer.send('hide-window'),
  directoryChooser: () => ipcRenderer.invoke('directory-chooser'),
  createChatWindow: (
    query?: string,
    dir?: string,
    version?: string,
    resumeSessionId?: string,
    viewType?: string,
    workflowId?: string
  ) =>
    ipcRenderer.send(
      'create-chat-window',
      query,
      dir,
      version,
      resumeSessionId,
      viewType,
      workflowId
    ),
  createDivergedChatWindow: (
    dir: string | undefined,
    resumeSessionId: string,
    resumeSessionTitle?: string
  ) => ipcRenderer.send('create-diverged-chat-window', dir, resumeSessionId, resumeSessionTitle),
  logInfo: (txt: string) => ipcRenderer.send('logInfo', txt),
  showNotification: (data: NotificationData) => ipcRenderer.send('notify', data),
  showMessageBox: (options: MessageBoxOptions) => ipcRenderer.invoke('show-message-box', options),
  showSaveDialog: (options: SaveDialogOptions) => ipcRenderer.invoke('show-save-dialog', options),
  saveDiagnosticsBundle: (sessionId: string, archive: ArrayBuffer) =>
    ipcRenderer.invoke('save-diagnostics-bundle', sessionId, archive),
  openInChrome: (url: string) => ipcRenderer.send('open-in-chrome', url),
  fetchMetadata: (url: string) => ipcRenderer.invoke('fetch-metadata', url),
  reloadApp: () => ipcRenderer.send('reload-app'),
  checkForOllama: () => ipcRenderer.invoke('check-ollama'),
  selectFileOrDirectory: (defaultPath?: string) =>
    ipcRenderer.invoke('select-file-or-directory', defaultPath),
  getBinaryPath: (binaryName: string) => ipcRenderer.invoke('get-binary-path', binaryName),
  importSessionFile: () => ipcRenderer.invoke('import-session-file'),
  readFile: (filePath: string) => ipcRenderer.invoke('read-file', filePath),
  readArtifactFile: (filePath: string) => ipcRenderer.invoke('read-artifact-file', filePath),
  checkFilePaths: (requests: FilePathCheckRequest[]) =>
    ipcRenderer.invoke('check-file-paths', requests),
  writeFile: (filePath: string, content: string) =>
    ipcRenderer.invoke('write-file', filePath, content),
  ensureDirectory: (dirPath: string) => ipcRenderer.invoke('ensure-directory', dirPath),
  listFiles: (dirPath: string, extension?: string) =>
    ipcRenderer.invoke('list-files', dirPath, extension),
  listSkillDirs: (dirPath: string) => ipcRenderer.invoke('list-skill-dirs', dirPath),
  deleteFile: (filePath: string) => ipcRenderer.invoke('delete-file', filePath),
  deleteDirectory: (dirPath: string) => ipcRenderer.invoke('delete-directory', dirPath),
  getPathForFile: (file: File) => webUtils.getPathForFile(file),
  getAllowedExtensions: () => ipcRenderer.invoke('get-allowed-extensions'),
  setMenuBarIcon: (show: boolean) => ipcRenderer.invoke('set-menu-bar-icon', show),
  getMenuBarIconState: () => ipcRenderer.invoke('get-menu-bar-icon-state'),
  setDockIcon: (show: boolean) => ipcRenderer.invoke('set-dock-icon', show),
  getDockIconState: () => ipcRenderer.invoke('get-dock-icon-state'),
  getSettings: () => ipcRenderer.invoke('get-settings'),
  saveSettings: (settings: unknown) => ipcRenderer.invoke('save-settings', settings),
  getSecretKey: () => ipcRenderer.invoke('get-secret-key'),
  getUserActionKey: () => ipcRenderer.invoke('get-user-action-key'),
  getBiorouterdHostPort: () => ipcRenderer.invoke('get-biorouterd-host-port'),
  setWakelock: (enable: boolean) => ipcRenderer.invoke('set-wakelock', enable),
  getWakelockState: () => ipcRenderer.invoke('get-wakelock-state'),
  setSpellcheck: (enable: boolean) => ipcRenderer.invoke('set-spellcheck', enable),
  getSpellcheckState: () => ipcRenderer.invoke('get-spellcheck-state'),
  openNotificationsSettings: () => ipcRenderer.invoke('open-notifications-settings'),
  onMouseBackButtonClicked: (callback: () => void) => {
    const wrapper = (_event: Electron.IpcRendererEvent) => callback();
    ipcRenderer.on('mouse-back-button-clicked', wrapper);
    return () => ipcRenderer.removeListener('mouse-back-button-clicked', wrapper);
  },
  offMouseBackButtonClicked: () => {
    warnOffDeprecated('mouse-back-button-clicked');
  },
  on: (
    channel: string,
    callback: (event: Electron.IpcRendererEvent, ...args: unknown[]) => void
  ) => {
    // Wrap in a preload-scope function so removeListener can match by
    // identity. Without this, contextBridge would hand `off()` a different
    // proxy than `on()` registered, and removal would silently no-op —
    // listeners would accumulate on every component remount.
    const wrapper = (event: Electron.IpcRendererEvent, ...args: unknown[]) =>
      callback(event, ...args);
    ipcRenderer.on(channel, wrapper);
    return () => ipcRenderer.removeListener(channel, wrapper);
  },
  off: (channel: string) => {
    warnOffDeprecated(channel);
  },
  emit: (channel: string, ...args: unknown[]) => {
    ipcRenderer.emit(channel, ...args);
  },
  broadcastThemeChange: (themeData: {
    mode: string;
    useSystemTheme: boolean;
    theme: string;
    themeFamily?: string;
  }) => {
    ipcRenderer.send('broadcast-theme-change', themeData);
  },
  saveDataUrlToTemp: (dataUrl: string, uniqueId: string): Promise<SaveDataUrlResponse> => {
    return ipcRenderer.invoke('save-data-url-to-temp', dataUrl, uniqueId);
  },
  deleteTempFile: (filePath: string): void => {
    ipcRenderer.send('delete-temp-file', filePath);
  },
  openExternal: (url: string): Promise<void> => {
    return ipcRenderer.invoke('open-external', url);
  },
  getTempImage: (filePath: string): Promise<string | null> => {
    return ipcRenderer.invoke('get-temp-image', filePath);
  },
  readTempImageAsBase64: (filePath: string): Promise<{ data: string; mimeType: string }> => {
    return ipcRenderer.invoke('read-temp-image-as-base64', filePath);
  },
  getVersion: (): string => {
    return config.BIOROUTER_VERSION || ipcRenderer.sendSync('get-app-version') || '';
  },
  checkForUpdates: (): Promise<{ updateInfo: unknown; error: string | null }> => {
    return ipcRenderer.invoke('check-for-updates');
  },
  downloadUpdate: (): Promise<{ success: boolean; error: string | null }> => {
    return ipcRenderer.invoke('download-update');
  },
  installUpdate: (): void => {
    ipcRenderer.invoke('install-update');
  },
  restartApp: (): void => {
    ipcRenderer.send('restart-app');
  },
  onUpdaterEvent: (callback: (event: UpdaterEvent) => void): (() => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: UpdaterEvent) => callback(data);
    ipcRenderer.on('updater-event', listener);
    return () => ipcRenderer.removeListener('updater-event', listener);
  },
  getUpdateState: (): Promise<{ updateAvailable: boolean; latestVersion?: string } | null> => {
    return ipcRenderer.invoke('get-update-state');
  },
  isUsingGitHubFallback: (): Promise<boolean> => {
    return ipcRenderer.invoke('is-using-github-fallback');
  },
  closeWindow: () => ipcRenderer.send('close-window'),
  windowDragStart: () => ipcRenderer.send('window:drag-start'),
  windowDragEnd: () => ipcRenderer.send('window:drag-end'),
  windowToggleZoom: () => ipcRenderer.send('window:toggle-zoom'),
  ensureWindowContentWidth: (width: number) =>
    ipcRenderer.invoke('window:ensure-content-width', width),
  hasAcceptedWorkflowBefore: (workflow: Workflow) =>
    ipcRenderer.invoke('has-accepted-workflow-before', workflow),
  recordWorkflowHash: (workflow: Workflow) => ipcRenderer.invoke('record-workflow-hash', workflow),
  openDirectoryInExplorer: (directoryPath: string) =>
    ipcRenderer.invoke('open-directory-in-explorer', directoryPath),
  captureRegion: (payload: {
    x: number;
    y: number;
    width: number;
    height: number;
    label?: string;
    containment?: { x: number; y: number; width: number; height: number };
  }) => ipcRenderer.invoke('capture-region', payload),
  embeddedBrowser: {
    isManagedAppUrl: (url: string) =>
      ipcRenderer.invoke('embedded-browser:is-managed-app-url', { url }),
    create: (viewId: string, url: string, managedOnly?: boolean) =>
      ipcRenderer.invoke('embedded-browser:create', { viewId, url, managedOnly }),
    setBounds: (viewId: string, bounds: { x: number; y: number; width: number; height: number }) =>
      ipcRenderer.invoke('embedded-browser:set-bounds', { viewId, bounds }),
    setVisible: (viewId: string, visible: boolean) =>
      ipcRenderer.invoke('embedded-browser:set-visible', { viewId, visible }),
    navigate: (viewId: string, url: string) =>
      ipcRenderer.invoke('embedded-browser:navigate', { viewId, url }),
    control: (viewId: string, action: 'back' | 'forward' | 'reload' | 'stop' | 'reload-if-idle') =>
      ipcRenderer.invoke('embedded-browser:control', { viewId, action }),
    destroy: (viewId: string) => ipcRenderer.invoke('embedded-browser:destroy', { viewId }),
    readText: (viewId: string, maxChars: number) =>
      ipcRenderer.invoke('embedded-browser:read-text', { viewId, maxChars }),
    capture: (viewId: string) => ipcRenderer.invoke('embedded-browser:capture', { viewId }),
    clearData: (viewId: string) => ipcRenderer.invoke('embedded-browser:clear-data', { viewId }),
    onState: (callback: (payload: { viewId: string; state: EmbeddedBrowserState }) => void) => {
      const wrapped = (_event: unknown, payload: { viewId: string; state: EmbeddedBrowserState }) =>
        callback(payload);
      ipcRenderer.on('embedded-browser:state', wrapped);
      return () => {
        ipcRenderer.removeListener('embedded-browser:state', wrapped);
      };
    },
  },
  openArtifactInBrowser: (payload: { html: string; title?: string; theme?: 'light' | 'dark' }) =>
    ipcRenderer.invoke('open-artifact-in-browser', payload),
  prepareArtifactHtml: (payload: { html: string }) =>
    ipcRenderer.invoke('prepare-artifact-html', payload),
  addRecentDir: (dir: string) => ipcRenderer.invoke('add-recent-dir', dir),
  openBrxtFilePicker: () => ipcRenderer.invoke('brxt:open-file-dialog'),
  validateBrxtBundle: (filePath: string) =>
    ipcRenderer.invoke('brxt:validate-and-read', { filePath }),
  installBrxtBundle: (
    filePath: string,
    extensionName: string,
    registrySource?: { registryId: string; sourceUrl?: string }
  ) => ipcRenderer.invoke('brxt:install', { filePath, extensionName, registrySource }),
  uninstallBrxtExtension: (extensionName: string) =>
    ipcRenderer.invoke('brxt:uninstall', { extensionName }),
  fetchRegistry: () => ipcRenderer.invoke('registry:fetch'),
  downloadRegistryAsset: (url: string) => ipcRenderer.invoke('registry:download', { url }),
  checkDependencies: (opts?: { force?: boolean }) => ipcRenderer.invoke('dep:check', opts),
  installDependency: (dep: string) => ipcRenderer.invoke('dep:install', dep),
  dependencyEnvironment: () => ipcRenderer.invoke('dep:environment'),
  cliStatus: () => ipcRenderer.invoke('cli:status'),
  installCli: () => ipcRenderer.invoke('cli:install'),
  launchCli: (workingDir?: string) => ipcRenderer.invoke('cli:launch', workingDir),
  createTerminalSession: (options?: { workingDir?: string; cols?: number; rows?: number }) =>
    ipcRenderer.invoke('terminal:create', options),
  writeTerminalSession: (sessionId: string, data: string) =>
    ipcRenderer.invoke('terminal:write', sessionId, data),
  resizeTerminalSession: (sessionId: string, cols: number, rows: number) =>
    ipcRenderer.invoke('terminal:resize', sessionId, cols, rows),
  disposeTerminalSession: (sessionId: string) => ipcRenderer.invoke('terminal:dispose', sessionId),
  onTerminalData: (callback) => {
    const listener = (_event: Electron.IpcRendererEvent, data: TerminalDataEvent) => callback(data);
    ipcRenderer.on('terminal:data', listener);
    return () => ipcRenderer.removeListener('terminal:data', listener);
  },
  onTerminalExit: (callback) => {
    const listener = (_event: Electron.IpcRendererEvent, data: TerminalExitEvent) => callback(data);
    ipcRenderer.on('terminal:exit', listener);
    return () => ipcRenderer.removeListener('terminal:exit', listener);
  },
  onExtensionUpdateEvent: (callback) => {
    // Named, preload-scope wrapper for the same reason the generic `on` above
    // has one: `removeListener` matches by identity, and an inline arrow passed
    // straight to `ipcRenderer.on` can never be handed back to remove it.
    const listener = (
      _event: Electron.IpcRendererEvent,
      data: import('./utils/extensionUpdater').ExtensionUpdateEvent
    ) => callback(data);
    ipcRenderer.on('extension-update-event', listener);
    return () => ipcRenderer.removeListener('extension-update-event', listener);
  },

  // Tab tear-off and merge. Append-only, and deliberately thin: every one of
  // these is a pass-through, because the geometry lives in `windowDrag.ts`
  // (main) and `tabTearOff.ts` (renderer) and preload is the wire, not a party.
  tabDragRegisterBands: (bands) => ipcRenderer.send('tab-drag:register-bands', bands),
  tabDragMove: (point) => ipcRenderer.send('tab-drag:move', point),
  tabDragCommit: (request) => ipcRenderer.invoke('tab-drag:commit', request),
  tabDragEnd: () => ipcRenderer.send('tab-drag:end'),
  tabDragAckMerge: (requestId, inserted) =>
    ipcRenderer.send('tab-drag:merge-ack', requestId, inserted),
};

const appConfigAPI: AppConfigAPI = {
  get: (key: string) => config[key],
  getAll: () => config,
};

// Expose the APIs
contextBridge.exposeInMainWorld('electron', electronAPI);
contextBridge.exposeInMainWorld('appConfig', appConfigAPI);

// Type declaration for TypeScript
declare global {
  interface Window {
    electron: ElectronAPI;
    appConfig: AppConfigAPI;
  }
}
