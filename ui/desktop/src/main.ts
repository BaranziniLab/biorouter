import type {
  MenuItemConstructorOptions,
  OpenDialogOptions,
  OpenDialogReturnValue,
  Rectangle,
} from 'electron';
import {
  app,
  App,
  BrowserWindow,
  dialog,
  globalShortcut,
  ipcMain,
  Menu,
  MenuItem,
  Notification,
  powerSaveBlocker,
  screen,
  session,
  shell,
  systemPreferences,
  Tray,
} from 'electron';
import { pathToFileURL, format as formatUrl, URLSearchParams } from 'node:url';
import { Buffer } from 'node:buffer';
import { isIP } from 'node:net';
import dns from 'node:dns/promises';
import fs from 'node:fs/promises';
import fsSync from 'node:fs';
import started from 'electron-squirrel-startup';
import path from 'node:path';
import os from 'node:os';
import { spawn, type ChildProcess } from 'child_process';
import AdmZip from 'adm-zip';
import { safeExtractZip } from './utils/safeZip';
import 'dotenv/config';
import { checkServerStatus, startBiorouterd, getBiorouterCliBinaryPath } from './biorouterd';
import {
  TerminalSessionRegistry,
  maxTerminalSessionsPerOwner,
  terminalSessionLimitMessage,
  type RegisteredTerminalSession,
} from './terminalSessionRegistry';
import { getSharedBackend, isSharedDaemonEnabled, resetSharedBackend } from './biorouterdSingleton';
import {
  StripBandRegistry,
  detachRefusal,
  electronScreenGeometry,
  grabOffsetFromWire,
  normalizeToDip,
  resolveDropTargetForRawPoint,
  screenPointFromWire,
  screenPointToWire,
  TabDragBroker,
  tornOffWindowBoundsForRawPoint,
  type Rect as DragRect,
  type ScreenGeometry,
} from './windowDrag';
import {
  doubleClickWindowAction,
  WindowMoveDragController,
  type MovableWindow,
} from './titlebarWindowGesture';
import {
  DragGhostWindowController,
  GHOST_OPAQUE_INSET,
  GHOST_PROBE_SCRIPT,
  GHOST_TRANSPARENT_INSET,
  ghostWindowDataUrl,
  ghostWindowHtml,
  type GhostSpec,
  type GhostWindowHandle,
} from './dragGhostWindow';
import { expandTilde, reinterpretTildeAsAbsolute } from './utils/pathUtils';
import { friendlyArtifactFileError } from './utils/artifactFileErrors';
import {
  assertSafeRasterImageDimensions,
  readFileHandleBounded,
  validateOfficeDocumentShape,
  validatedOfficeZip,
} from './utils/artifactPreviewLimits';
import { artifactSourceRevision } from './utils/artifactSourceRevision';
import { sanitizeUntrustedLabel } from './utils/untrustedText';
import { inlineArtifactCdnAssets } from './utils/artifactCdnAssets';
import { isFilePathAllowedForPreview, previewFileRoots } from './utils/pathContainment';
import { findBrxtArgument, isBrxtFile } from './utils/launchArguments';
import log from './utils/logger';
import { ensureWinShims } from './utils/winShims';
import { addRecentDir, loadRecentDirs } from './utils/recentDirs';
import {
  EnvToggles,
  loadSettings,
  saveSettings,
  updateEnvironmentVariables,
} from './utils/settings';
import * as crypto from 'crypto';
// import electron from "electron";
import * as yaml from 'yaml';
import windowStateKeeper from 'electron-window-state';
import {
  getUpdateAvailable,
  openUpdateSettings,
  popUpTrayMenu,
  registerUpdateIpcHandlers,
  setTrayRef,
  setupAutoUpdater,
  updateTrayMenu,
} from './utils/autoUpdater';
import { UPDATES_ENABLED } from './updates';
import { startMainThreadWatchdog } from './utils/mainThreadWatchdog';
import {
  STARTUP_UPDATER_SETUP_DELAY_MS,
  STARTUP_DEPENDENCY_CHECK_DELAY_MS,
  STARTUP_EXTENSION_CHECK_DELAY_MS,
} from './utils/startupSchedule';
import './utils/workflowHash';
import { parseWorkflowDeeplink, type WorkflowDeeplinkData } from './utils/workflowDeeplink';
import {
  registerDependencyIpcHandlers,
  setupDependencyChecker,
  triggerDependencyCheck,
  invalidateDependencyCache,
  runProbe,
  SPAWN_ENV,
} from './utils/dependencyChecker';
import { runExtensionUpdateCheck, scheduleExtensionUpdateCheck } from './utils/extensionUpdater';
import {
  isAllowedArtifactFrameNavigation,
  isAllowedRendererPermission,
  isAppOrigin,
  shouldOpenExternalNavigation,
} from './utils/permissionPolicy';
import {
  ARTIFACT_WRAPPER_CSP,
  injectArtifactHostTheme,
  wrapArtifactForBrowser,
} from './utils/artifactSecurity';
import { readGitArtifactTree } from './utils/artifactGit';
import {
  captureEmbeddedBrowser,
  clearEmbeddedBrowserData,
  controlEmbeddedBrowser,
  createEmbeddedBrowser,
  destroyEmbeddedBrowser,
  destroyEmbeddedBrowsersForWindow,
  navigateEmbeddedBrowser,
  openExternalBrowserNavigation,
  readEmbeddedBrowserText,
  registerEmbeddedBrowserOwnerTeardown,
  setEmbeddedBrowserBounds,
  setEmbeddedBrowserVisible,
  type EmbeddedBrowserBounds,
} from './utils/embeddedBrowser';
import { heicToPng } from './utils/heicConvert';
import { bindManagedAppPreviewBackend } from './utils/managedAppPreviewBackend';
import {
  managedAppPreviewScope,
  type ManagedAppPreviewBackend,
} from './utils/managedAppPreviewPolicy';
import { IMAGE_BLOB_URL_THRESHOLD_BYTES, IMAGE_MIME_TYPES } from './utils/imageFormats';
import { recordExtensionProvenance } from './utils/extensionProvenance';
import {
  biorouterConfigDir,
  biorouterExtensionsDir,
  unsandboxedConfigDirCandidates,
} from './utils/biorouterPaths';
import { fetchRegistryWithLastGood } from './utils/registryCache';
import { readArtifactDirectoryTree } from './utils/artifactDirectory';
import {
  diagnosticsArchiveBytes,
  diagnosticsArchiveFilename,
  type DiagnosticsArchivePayload,
} from './utils/diagnosticsExport';
import { Client, createClient, createConfig } from './api/client';
import installExtension, { REACT_DEVELOPER_TOOLS } from 'electron-devtools-installer';

// Updater functions (moved here to keep updates.ts minimal for release replacement)
function shouldSetupUpdater(): boolean {
  // Setup updater if either the flag is enabled OR dev updates are enabled
  return UPDATES_ENABLED || process.env.ENABLE_DEV_UPDATES === 'true';
}

// Define temp directory for pasted images
const biorouterTempDir = path.join(app.getPath('temp'), 'biorouter-pasted-images');

function resolveImagePath(filename: string): string | undefined {
  return [
    path.join(process.resourcesPath, 'images', filename),
    path.join(process.cwd(), 'src', 'images', filename),
    path.join(__dirname, '..', 'images', filename),
    path.join(__dirname, 'images', filename),
    path.join(process.cwd(), 'images', filename),
  ].find((candidate) => fsSync.existsSync(candidate));
}

function expandBiorouterPath(filePath: string): string {
  // `reinterpretTildeAsAbsolute` recovers `~/ws/…` when the chat's working
  // directory is outside the home tree; it is a no-op whenever the home reading
  // exists, so it can never redirect a path that already works.
  const expandedPath = reinterpretTildeAsAbsolute(filePath, expandTilde(filePath), (candidate) =>
    fsSync.existsSync(candidate)
  );
  const pathRoot = process.env.BIOROUTER_PATH_ROOT;
  if (!pathRoot || !pathRoot.trim()) return expandedPath;

  // Both spellings of "the real config directory" are redirected — see
  // `unsandboxedConfigDirCandidates`. Under a default XDG setup they are one
  // and the same, so this loop runs once and behaves exactly as the single
  // hardcoded join it replaced.
  for (const configDir of unsandboxedConfigDirCandidates()) {
    if (expandedPath === configDir || expandedPath.startsWith(configDir + path.sep)) {
      return path.join(pathRoot, 'config', path.relative(configDir, expandedPath));
    }
  }
  return expandedPath;
}

/**
 * @param sessionWorkingDir the working directory of the window making the
 *   request, when known. **Must come from the main process's own record of the
 *   window** (see `windowWorkingDirs`), never from an IPC argument — a renderer
 *   that could name its own root would void the boundary entirely, which is why
 *   the "paths the task touched" registry mentioned below was rejected.
 */
export function allowedFileRoots(sessionWorkingDir?: string): string[] {
  // Thin wrapper: the SET lives in utils/pathContainment.ts so it is testable
  // (nothing can import main.ts under vitest). The comment about why a
  // renderer-declared "paths the task touched" registry was rejected still
  // applies — `sessionWorkingDir` must come from `windowWorkingDirs`, which
  // main populates itself when it builds the window.
  return previewFileRoots({
    sessionWorkingDir,
    home: os.homedir(),
    userData: app.getPath('userData'),
    appTemp: app.getPath('temp'),
    systemTemp: os.tmpdir(),
    platform: process.platform,
    pathRootOverride: process.env.BIOROUTER_PATH_ROOT,
  });
}

/** The biorouter config.yaml path in the main process. Resolved by
 *  `utils/biorouterPaths.ts`, which is the ONE derivation in this process —
 *  it honours the BIOROUTER_PATH_ROOT redirect and, unlike the hardcoded join
 *  that used to sit here, `XDG_CONFIG_HOME` and the Windows layout too. */
function biorouterConfigYamlPath(): string {
  return path.join(biorouterConfigDir(), 'config.yaml');
}

// The preview allowlist must know the permission mode, but MUST read it from the
// config file in the MAIN process — never trust a renderer IPC message to
// declare it, or a compromised renderer could unlock the whole filesystem.
// Cached briefly so a burst of preview reads does not re-parse the file; a
// settings change is picked up within the TTL. Fail-closed: an unreadable
// config yields '' (not 'auto'), keeping the narrow home/temp allowlist.
let cachedBiorouterMode: { value: string; at: number } | null = null;
const BIOROUTER_MODE_CACHE_MS = 1500;

function readBiorouterMode(): string {
  const now = Date.now();
  if (cachedBiorouterMode && now - cachedBiorouterMode.at <= BIOROUTER_MODE_CACHE_MS) {
    return cachedBiorouterMode.value;
  }
  let value = '';
  try {
    const raw = fsSync.readFileSync(biorouterConfigYamlPath(), 'utf8');
    const parsed = yaml.parse(raw) as Record<string, unknown> | null;
    const mode = parsed?.BIOROUTER_MODE;
    if (typeof mode === 'string') value = mode.trim().toLowerCase();
  } catch {
    value = '';
  }
  cachedBiorouterMode = { value, at: now };
  return value;
}

export function isFullyAutomaticMode(): boolean {
  return readBiorouterMode() === 'auto';
}

export function isAllowedFilePath(resolvedPath: string, sessionWorkingDir?: string): boolean {
  // Symlink-aware containment + Directive 2 mode-aware scope: Fully-Automatic
  // mode admits any non-sensitive path (parity with the backend, which lets the
  // agent write anywhere); sensitive paths stay denied regardless of mode. See
  // utils/pathContainment.ts.
  //
  // A sensitive path stays denied even when it IS the working directory: the
  // widening adds a root to the allowlist, it does not bypass
  // `isSensitivePreviewPath`. Pointing a chat at ~/.ssh does not make it
  // readable.
  return isFilePathAllowedForPreview(resolvedPath, allowedFileRoots(sessionWorkingDir), {
    fullyAutomatic: isFullyAutomaticMode(),
  });
}

/**
 * Per-window working directory, as the MAIN process recorded it when it built
 * the window. This is the trusted source for `isAllowedFilePath`'s widening —
 * `windowWorkingDir` is computed in `createChat` from the launch argument, not
 * received over IPC.
 */
const windowWorkingDirs = new Map<number, string>();

/** The working directory of the window that sent `event`, if it has one. */
export function workingDirForSender(event: { sender: Electron.WebContents }): string | undefined {
  const win = BrowserWindow.fromWebContents(event.sender);
  return win ? windowWorkingDirs.get(win.id) : undefined;
}

/**
 * True only for a directory that is genuinely a *folder* — something a file
 * manager opens and nothing else.
 *
 * `stats.isDirectory()` on its own is not that test. A macOS package
 * (`.app`, `.pkg`, `.workflow`, `.rtfd`, …) stats as a directory, and `open`ing
 * one LAUNCHES or installs it, so a bare `isDirectory()` waves every bundle
 * straight through to execution. Rather than carry a list of package
 * extensions — a denylist, which fails open on the one nobody thought of —
 * anything carrying an extension is treated as not-a-plain-folder and takes the
 * confirmation below. The folders this handler actually opens (a working
 * directory, a skill directory, a knowledge base) have no extension, so the
 * common path is unchanged.
 */
async function isPlainDirectory(resolvedPath: string): Promise<boolean> {
  try {
    const stats = await fs.stat(resolvedPath);
    return stats.isDirectory() && path.extname(resolvedPath) === '';
  } catch {
    return false;
  }
}

/**
 * Native confirmation before a path is handed to the OS's default handler.
 *
 * The asymmetry this removes: `open-external` will not let a URL reach the
 * system browser without `validateExternalBrowserTarget` AND a dialog naming
 * the destination, while a FILE reached the system browser — or a shell, or an
 * installer — with no check at all. Once handed over, a generated `.html` runs
 * as `file://` with no CSP and no sandbox, which is precisely what the artifact
 * panel's own iframe exists to prevent. The path is chosen by the agent, so the
 * user is the one who decides.
 *
 * Same shape as `confirmPublicExternalNavigation`: name the exact target,
 * default to Cancel.
 */
async function confirmSystemHandlerOpen(
  event: { sender: Electron.WebContents },
  resolvedPath: string
): Promise<boolean> {
  const options: Electron.MessageBoxOptions = {
    type: 'question',
    buttons: ['Cancel', 'Open'],
    defaultId: 0,
    cancelId: 0,
    noLink: true,
    message: 'Open this with the app your system uses for it?',
    detail:
      `${resolvedPath}\n\n` +
      'Biorouter hands this to your operating system, which decides what runs. ' +
      'A web page opened this way runs outside the app sandbox.',
  };
  const window = BrowserWindow.fromWebContents(event.sender);
  const result =
    window && !window.isDestroyed()
      ? await dialog.showMessageBox(window, options)
      : await dialog.showMessageBox(options);
  return result.response === 1;
}

/**
 * Reject addresses that only exist inside the user's machine or LAN: the
 * biorouterd loopback API, cloud metadata at 169.254.169.254, printers, routers.
 */
export function isPrivateAddress(address: string): boolean {
  const family = isIP(address);
  if (family === 4) {
    const [a, b] = address.split('.').map(Number);
    if (a === 0 || a === 127 || a === 10) return true;
    if (a === 172 && b >= 16 && b <= 31) return true;
    if (a === 192 && b === 168) return true;
    if (a === 169 && b === 254) return true; // link-local, incl. cloud metadata
    if (a === 100 && b >= 64 && b <= 127) return true; // CGNAT
    if (a >= 224) return true; // multicast / reserved / broadcast
    return false;
  }
  if (family === 6) {
    const addr = address.toLowerCase();
    if (addr === '::' || addr === '::1') return true;
    if (addr.startsWith('fe8') || addr.startsWith('fe9')) return true;
    if (addr.startsWith('fea') || addr.startsWith('feb')) return true; // fe80::/10
    if (addr.startsWith('fc') || addr.startsWith('fd')) return true; // fc00::/7 ULA
    const mapped = addr.match(/^::ffff:(\d+\.\d+\.\d+\.\d+)$/);
    if (mapped) return isPrivateAddress(mapped[1]);
    return false;
  }
  return false;
}

/** Throws unless `candidate` is an http(s) URL whose host resolves off-machine. */
async function assertPublicHttpUrl(candidate: string): Promise<URL> {
  const parsed = new URL(candidate);
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new Error('Invalid URL protocol. Only HTTP and HTTPS are allowed.');
  }
  const host = parsed.hostname.replace(/^\[|\]$/g, '');
  if (isIP(host)) {
    if (isPrivateAddress(host)) throw new Error(`Blocked non-public address: ${host}`);
    return parsed;
  }
  const resolved = await dns.lookup(host, { all: true });
  if (resolved.some((entry) => isPrivateAddress(entry.address))) {
    throw new Error(`Blocked non-public address for host: ${host}`);
  }
  return parsed;
}

/** Largest artifact the previewer will read into memory. */
const ARTIFACT_PREVIEW_MAX_BYTES = 16 * 1024 * 1024;
const OFFICE_TEXT_MAX_CHARS = 100_000;

function decodeOfficeXmlText(value: string): string {
  return value
    .replace(/<w:tab\s*\/?\s*>/g, '\t')
    .replace(/<w:br\s*\/?\s*>/g, '\n')
    .replace(/<\/w:p>|<\/a:p>|<\/row>/g, '\n')
    .replace(/<[^>]+>/g, '')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, '&')
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

function extractOfficeText(
  zip: AdmZip,
  format: 'docx' | 'xlsx' | 'pptx'
): { text: string; truncated: boolean } {
  let text = '';
  if (format === 'docx') {
    const names = zip
      .getEntries()
      .map((entry) => entry.entryName)
      .filter((name) =>
        /^word\/(document|header\d+|footer\d+|footnotes|endnotes)\.xml$/.test(name)
      );
    text = names
      .map((name) => decodeOfficeXmlText(zip.getEntry(name)?.getData().toString('utf8') ?? ''))
      .filter(Boolean)
      .join('\n\n');
  } else if (format === 'pptx') {
    const slides = zip
      .getEntries()
      .map((entry) => entry.entryName)
      .filter((name) => /^ppt\/slides\/slide\d+\.xml$/.test(name))
      .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
    text = slides
      .map(
        (name, index) =>
          `[Slide ${index + 1}]\n${decodeOfficeXmlText(zip.getEntry(name)?.getData().toString('utf8') ?? '')}`
      )
      .join('\n\n');
  } else {
    const sharedXml = zip.getEntry('xl/sharedStrings.xml')?.getData().toString('utf8') ?? '';
    const shared = [...sharedXml.matchAll(/<si\b[^>]*>([\s\S]*?)<\/si>/g)].map((match) =>
      decodeOfficeXmlText(match[1])
    );
    const sheets = zip
      .getEntries()
      .map((entry) => entry.entryName)
      .filter((name) => /^xl\/worksheets\/sheet\d+\.xml$/.test(name))
      .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
    const rows: string[] = [];
    for (const [sheetIndex, name] of sheets.entries()) {
      rows.push(`[Sheet ${sheetIndex + 1}]`);
      const xml = zip.getEntry(name)?.getData().toString('utf8') ?? '';
      for (const cell of xml.matchAll(/<c\b([^>]*)>([\s\S]*?)<\/c>/g)) {
        if (rows.length >= 10_000) break;
        const reference = cell[1].match(/\br="([^"]+)"/)?.[1] ?? '?';
        const kind = cell[1].match(/\bt="([^"]+)"/)?.[1];
        const raw = cell[2].match(/<v>([\s\S]*?)<\/v>/)?.[1];
        const inline = cell[2].match(/<is>([\s\S]*?)<\/is>/)?.[1];
        const value =
          kind === 's' && raw ? shared[Number(raw)] : decodeOfficeXmlText(inline ?? raw ?? '');
        if (value) rows.push(`${reference}: ${value}`);
      }
    }
    text = rows.join('\n');
  }

  return {
    text: text.slice(0, OFFICE_TEXT_MAX_CHARS),
    truncated: text.length > OFFICE_TEXT_MAX_CHARS,
  };
}

/** Only http(s) may be handed to the OS opener. */
export function isExternallyOpenableUrl(candidate: string): boolean {
  if (candidate.length > 8 * 1024) return false;
  try {
    const url = new URL(candidate);
    return (
      (url.protocol === 'http:' || url.protocol === 'https:') &&
      url.username === '' &&
      url.password === ''
    );
  } catch {
    return false;
  }
}

function rendererEntryUrl(): URL {
  return MAIN_WINDOW_VITE_DEV_SERVER_URL
    ? new URL(MAIN_WINDOW_VITE_DEV_SERVER_URL)
    : pathToFileURL(path.join(__dirname, `../renderer/${MAIN_WINDOW_VITE_NAME}/index.html`));
}

// Image entries are spread in from `utils/imageFormats` rather than written out
// here. This map is the one that decides `kind: 'image'` (via the
// `startsWith('image/')` test in the artifact read handler), so a format missing
// from it is a format the panel silently treats as an opaque binary — which is
// exactly how `bmp`, `ico` and `avif` came to be unsupported despite Chromium
// having decoded all three for years.
const ARTIFACT_MIME_TYPES: Record<string, string> = {
  ...Object.fromEntries(
    Object.entries(IMAGE_MIME_TYPES).map(([extension, mime]) => [`.${extension}`, mime])
  ),
  '.css': 'text/css',
  '.csv': 'text/csv',
  '.htm': 'text/html',
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.json': 'application/json',
  '.ipynb': 'application/x-ipynb+json',
  '.md': 'text/markdown',
  '.pdf': 'application/pdf',
  '.docx': 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  '.pptx': 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  '.xlsx': 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  '.py': 'text/x-python',
  '.r': 'text/x-r',
  '.rs': 'text/rust',
  '.sh': 'text/x-shellscript',
  '.sql': 'application/sql',
  '.toml': 'text/toml',
  '.ts': 'text/typescript',
  '.tsx': 'text/typescript',
  '.txt': 'text/plain',
  '.xml': 'application/xml',
  '.yaml': 'application/yaml',
  '.yml': 'application/yaml',
};

function mimeTypeForArtifactPath(filePath: string): string {
  const ext = path.extname(filePath).toLowerCase();
  return ARTIFACT_MIME_TYPES[ext] || 'application/octet-stream';
}

function documentFormatForArtifactPath(filePath: string) {
  const formats = {
    '.pdf': 'pdf',
    '.docx': 'docx',
    '.xlsx': 'xlsx',
    '.pptx': 'pptx',
  } as const;
  return formats[path.extname(filePath).toLowerCase() as keyof typeof formats] ?? null;
}

function isTextArtifact(mimeType: string, buffer: Buffer): boolean {
  if (
    mimeType.startsWith('text/') ||
    mimeType.includes('json') ||
    mimeType.includes('xml') ||
    mimeType.includes('yaml') ||
    mimeType.includes('sql')
  ) {
    return true;
  }
  return !buffer.subarray(0, Math.min(buffer.length, 512)).includes(0);
}

// Function to ensure the temporary directory exists
async function ensureTempDirExists(): Promise<string> {
  try {
    // Check if the path already exists
    try {
      const stats = await fs.stat(biorouterTempDir);

      // If it exists but is not a directory, remove it and recreate
      if (!stats.isDirectory()) {
        await fs.unlink(biorouterTempDir);
        await fs.mkdir(biorouterTempDir, { recursive: true, mode: 0o700 });
      }

      // Startup cleanup: remove old files and any symlinks
      const files = await fs.readdir(biorouterTempDir);
      const now = Date.now();
      const MAX_AGE = 24 * 60 * 60 * 1000; // 24 hours in milliseconds

      for (const file of files) {
        const filePath = path.join(biorouterTempDir, file);
        try {
          const fileStats = await fs.lstat(filePath);

          // Always remove symlinks
          if (fileStats.isSymbolicLink()) {
            console.warn(
              `[Main] Found symlink in temp directory during startup: ${filePath}. Removing it.`
            );
            await fs.unlink(filePath);
            continue;
          }

          // Remove old files (older than 24 hours)
          if (fileStats.isFile()) {
            const fileAge = now - fileStats.mtime.getTime();
            if (fileAge > MAX_AGE) {
              console.log(
                `[Main] Removing old temp file during startup: ${filePath} (age: ${Math.round(fileAge / (60 * 60 * 1000))} hours)`
              );
              await fs.unlink(filePath);
            }
          }
        } catch (fileError) {
          // If we can't stat the file, try to remove it anyway
          console.warn(`[Main] Could not stat file ${filePath}, attempting to remove:`, fileError);
          try {
            await fs.unlink(filePath);
          } catch (unlinkError) {
            console.error(`[Main] Failed to remove problematic file ${filePath}:`, unlinkError);
          }
        }
      }
    } catch (error) {
      if (error && typeof error === 'object' && 'code' in error && error.code === 'ENOENT') {
        // Directory doesn't exist, create it
        await fs.mkdir(biorouterTempDir, { recursive: true, mode: 0o700 });
      } else {
        throw error;
      }
    }

    await fs.chmod(biorouterTempDir, 0o700);

    console.log('[Main] Temporary directory for pasted images ensured:', biorouterTempDir);
  } catch (error) {
    console.error('[Main] Failed to create temp directory:', biorouterTempDir, error);
    throw error; // Propagate error
  }
  return biorouterTempDir;
}

async function configureProxy() {
  const httpsProxy = process.env.HTTPS_PROXY || process.env.https_proxy;
  const httpProxy = process.env.HTTP_PROXY || process.env.http_proxy;
  const noProxy = process.env.NO_PROXY || process.env.no_proxy || '';

  const proxyUrl = httpsProxy || httpProxy;

  if (proxyUrl) {
    console.log('[Main] Configuring proxy');
    await session.defaultSession.setProxy({
      proxyRules: proxyUrl,
      proxyBypassRules: noProxy,
    });
    console.log('[Main] Proxy configured successfully');
  }
}

if (started) app.quit();

// Global safety net: turn uncaught exceptions / unhandled rejections in the
// main process into logged diagnostics instead of bare brk #0 aborts. The
// default Node behavior tears the process down with no readable cause line
// in the crash report, which is what every recent Biorouter crash report
// has looked like. Logging here doesn't *prevent* the crash, but it
// guarantees the next .ips will have actionable context.
process.on('uncaughtException', (err, origin) => {
  try {
    log.error(`[Main] uncaughtException (${origin}):`, err);
  } catch {
    /* logger itself may be torn down — swallow */
  }
});
process.on('unhandledRejection', (reason) => {
  try {
    log.error('[Main] unhandledRejection:', reason);
  } catch {
    /* logger itself may be torn down — swallow */
  }
});

if (process.env.ENABLE_PLAYWRIGHT) {
  const cdpPort = process.env.PLAYWRIGHT_CDP_PORT ?? '9222';
  console.log(`[Main] Enabling Playwright remote debugging on port ${cdpPort}`);
  app.commandLine.appendSwitch('remote-debugging-port', cdpPort);
}

// Register as the handler for biorouter:// deep links.
//
// On macOS this maps the scheme to the *running bundle's* identifier. In a dev
// tree that bundle is `node_modules/electron/dist/Electron.app`
// (`com.github.Electron`) — a bare Electron shell with no app to run. Claiming
// the scheme from there permanently steals `biorouter://` from the installed
// app, and every subsequent link launches the shell, which exits immediately.
// So on macOS we only register from a packaged build.
//
// Windows/Linux resolve the handler by executable path rather than bundle id,
// and Electron's documented dev form (execPath + the app entry point) launches
// the real app, so registering there is both safe and useful.
if (process.platform === 'darwin') {
  if (app.isPackaged) {
    app.setAsDefaultProtocolClient('biorouter');
  } else {
    log.info(
      '[Main] Dev build on macOS: skipping biorouter:// registration so the installed app keeps the scheme'
    );
  }
} else if (app.isPackaged || !process.argv[1]) {
  app.setAsDefaultProtocolClient('biorouter');
} else {
  app.setAsDefaultProtocolClient('biorouter', process.execPath, [path.resolve(process.argv[1])]);
}

// Set as soon as we know a deep link is driving this launch, so appMain() does
// not also open an empty window. Declared here because the Windows/Linux argv
// path below claims the launch synchronously.
let openUrlHandledLaunch = false;

/** Deep links that open their own window rather than reusing an existing one. */
const WINDOW_OWNING_DEEPLINK_HOSTS = ['bot', 'workflow', 'diverge'];

// Apply single instance lock on Windows and Linux where it's needed for deep links
// macOS uses the 'open-url' event instead
let gotTheLock = true;
if (process.platform !== 'darwin') {
  gotTheLock = app.requestSingleInstanceLock();

  if (!gotTheLock) {
    app.quit();
  } else {
    app.on('second-instance', (_event, commandLine) => {
      const protocolUrl = commandLine.find((arg) => arg.startsWith('biorouter://'));
      if (protocolUrl) {
        let parsedUrl: URL;
        try {
          parsedUrl = new URL(protocolUrl);
        } catch (error) {
          log.error('[Main] Ignoring malformed deep link:', protocolUrl, error);
          return;
        }
        // Diverge: always open the branch in a fresh, focused window.
        if (parsedUrl.hostname === 'diverge') {
          app.whenReady().then(() => openDivergedWindow(parsedUrl));
          return;
        }
        // If it's a bot/workflow URL, handle it directly by creating a new window
        if (parsedUrl.hostname === 'bot' || parsedUrl.hostname === 'workflow') {
          app.whenReady().then(async () => {
            const recentDirs = loadRecentDirs();
            const openDir = recentDirs.length > 0 ? recentDirs[0] : null;

            const deeplinkData = parseWorkflowDeeplink(protocolUrl);
            const scheduledJobId = parsedUrl.searchParams.get('scheduledJob');

            createChat(
              app,
              undefined,
              openDir || undefined,
              undefined,
              undefined,
              undefined,
              deeplinkData?.config,
              scheduledJobId || undefined,
              undefined,
              deeplinkData?.parameters
            );
          });
          return; // Skip the rest of the handler
        }

        // For non-bot URLs, continue with normal handling
        handleProtocolUrl(protocolUrl);
      }

      const brxtArg = findBrxtArgument(commandLine);
      if (brxtArg) {
        app.whenReady().then(() => handleBrxtFileOpen(brxtArg));
      }

      // Only focus existing windows for non-bot/workflow URLs
      const existingWindows = BrowserWindow.getAllWindows();
      if (existingWindows.length > 0) {
        const mainWindow = existingWindows[0];
        if (mainWindow.isMinimized()) {
          mainWindow.restore();
        }
        mainWindow.focus();
      }
    });
  }

  // Handle protocol URLs on Windows and Linux startup
  const protocolUrl = process.argv.find((arg) => arg.startsWith('biorouter://'));
  if (protocolUrl) {
    try {
      if (WINDOW_OWNING_DEEPLINK_HOSTS.includes(new URL(protocolUrl).hostname)) {
        openUrlHandledLaunch = true;
      }
    } catch (error) {
      log.error('[Main] Ignoring malformed deep link argument:', protocolUrl, error);
    }
    app.whenReady().then(() => {
      handleProtocolUrl(protocolUrl);
    });
  }

  // Check if launched with a .brxt file argument (Windows/Linux double-click)
  const brxtArg = findBrxtArgument(process.argv.slice(1));
  if (brxtArg) {
    app.whenReady().then(() => handleBrxtFileOpen(brxtArg));
  }
}

let firstOpenWindow: BrowserWindow;
let pendingDeepLink: string | null = null;
let pendingBrxtFilePath: string | null = null;

/**
 * A window-owning deep link claims the launch, so appMain() will not open its
 * own window. If the link then fails to produce one — a malformed URL, a
 * backend that won't start — the app would sit running with nothing on screen,
 * which is indistinguishable from "clicking the link quit Biorouter". Always
 * leave the user with a window.
 */
async function ensureWindowAfterDeepLink(openDir?: string | null) {
  if (BrowserWindow.getAllWindows().length > 0) return;
  log.warn('[Main] Deep link produced no window; opening a plain one instead');
  await createNewWindow(app, openDir || undefined);
}

async function handleProtocolUrl(url: string) {
  if (!url) return;

  pendingDeepLink = url;

  let parsedUrl: URL;
  try {
    parsedUrl = new URL(url);
  } catch (error) {
    log.error('[Main] Ignoring malformed deep link:', url, error);
    pendingDeepLink = null;
    await ensureWindowAfterDeepLink();
    return;
  }
  const recentDirs = loadRecentDirs();
  const openDir = recentDirs.length > 0 ? recentDirs[0] : null;

  // Diverge: always open the branch in a fresh, focused window.
  if (parsedUrl.hostname === 'diverge') {
    pendingDeepLink = null;
    try {
      await openDivergedWindow(parsedUrl);
    } catch (error) {
      log.error('[Main] Failed to open diverge deep link:', error);
    }
    await ensureWindowAfterDeepLink(openDir);
    return;
  }

  if (parsedUrl.hostname === 'bot' || parsedUrl.hostname === 'workflow') {
    // processProtocolUrl always opens its own window for these, so don't create
    // a throwaway one first — that left a stray empty window on cold launches.
    try {
      await processProtocolUrl(parsedUrl, null);
    } catch (error) {
      log.error('[Main] Failed to open workflow deep link:', error);
    }
    await ensureWindowAfterDeepLink(openDir);
  } else {
    // For other URL types, reuse existing window if available
    const existingWindows = BrowserWindow.getAllWindows();
    if (existingWindows.length > 0) {
      firstOpenWindow = existingWindows[0];
      if (firstOpenWindow.isMinimized()) {
        firstOpenWindow.restore();
      }
      firstOpenWindow.focus();
    } else {
      firstOpenWindow = await createChat(app, undefined, openDir || undefined);
    }

    if (firstOpenWindow) {
      const webContents = firstOpenWindow.webContents;
      if (webContents.isLoadingMainFrame()) {
        webContents.once('did-finish-load', async () => {
          await processProtocolUrl(parsedUrl, firstOpenWindow);
        });
      } else {
        await processProtocolUrl(parsedUrl, firstOpenWindow);
      }
    }
  }
}

// `window` is null for bot/workflow URLs, which always open a window of their own.
async function processProtocolUrl(parsedUrl: URL, window: BrowserWindow | null) {
  const recentDirs = loadRecentDirs();
  const openDir = recentDirs.length > 0 ? recentDirs[0] : null;

  if (parsedUrl.hostname === 'extension') {
    window?.webContents.send('add-extension', pendingDeepLink);
  } else if (parsedUrl.hostname === 'sessions') {
    window?.webContents.send('open-shared-session', pendingDeepLink);
  } else if (parsedUrl.hostname === 'bot' || parsedUrl.hostname === 'workflow') {
    const deeplinkData = parseWorkflowDeeplink(pendingDeepLink ?? parsedUrl.toString());
    const scheduledJobId = parsedUrl.searchParams.get('scheduledJob');

    // Opens its own window; the `window` argument is deliberately unused here.
    // Awaited so callers can tell whether a window actually appeared.
    await createChat(
      app,
      undefined,
      openDir || undefined,
      undefined,
      undefined,
      undefined,
      deeplinkData?.config,
      scheduledJobId || undefined,
      undefined,
      deeplinkData?.parameters
    );
    pendingDeepLink = null;
  }
}

let windowDeeplinkURL: string | null = null;

app.on('open-url', async (_event, url) => {
  if (process.platform !== 'win32') {
    let parsedUrl: URL;
    try {
      parsedUrl = new URL(url);
    } catch (error) {
      log.error('[Main] Ignoring malformed deep link:', url, error);
      return;
    }

    log.info('[Main] Received open-url event:', url);

    // On a cold launch macOS emits open-url before `ready`, so this handler and
    // appMain both wait on the same whenReady() promise — and appMain's
    // continuation was queued first. Claim the launch *now*, synchronously,
    // otherwise appMain sees the flag still false and opens a redundant empty
    // window alongside the one this handler is about to create.
    if (!app.isReady() && WINDOW_OWNING_DEEPLINK_HOSTS.includes(parsedUrl.hostname)) {
      openUrlHandledLaunch = true;
    }

    await app.whenReady();

    const recentDirs = loadRecentDirs();
    const openDir = recentDirs.length > 0 ? recentDirs[0] : null;

    // Diverge: always open the branch in a fresh, focused window.
    if (parsedUrl.hostname === 'diverge') {
      log.info('[Main] Detected diverge URL, opening branch in a new window');
      openUrlHandledLaunch = true;
      try {
        await openDivergedWindow(parsedUrl);
      } catch (error) {
        log.error('[Main] Failed to open diverge deep link:', error);
      }
      await ensureWindowAfterDeepLink(openDir);
      return;
    }

    // Handle bot/workflow URLs by directly creating a new window
    if (parsedUrl.hostname === 'bot' || parsedUrl.hostname === 'workflow') {
      log.info('[Main] Detected bot/workflow URL, creating new chat window');
      openUrlHandledLaunch = true;
      const deeplinkData = parseWorkflowDeeplink(url);
      if (deeplinkData) {
        windowDeeplinkURL = url;
      }
      const scheduledJobId = parsedUrl.searchParams.get('scheduledJob');

      try {
        await createChat(
          app,
          undefined,
          openDir || undefined,
          undefined,
          undefined,
          undefined,
          deeplinkData?.config,
          scheduledJobId || undefined,
          undefined,
          deeplinkData?.parameters
        );
      } catch (error) {
        log.error('[Main] Failed to open workflow deep link:', error);
      } finally {
        windowDeeplinkURL = null;
      }
      await ensureWindowAfterDeepLink(openDir);
      return;
    }

    // For extension/session URLs, store the deep link for processing after React is ready
    pendingDeepLink = url;
    log.info('[Main] Stored pending deep link for processing after React ready:', url);

    const existingWindows = BrowserWindow.getAllWindows();
    if (existingWindows.length > 0) {
      firstOpenWindow = existingWindows[0];
      if (firstOpenWindow.isMinimized()) firstOpenWindow.restore();
      firstOpenWindow.focus();
      if (parsedUrl.hostname === 'extension') {
        firstOpenWindow.webContents.send('add-extension', pendingDeepLink);
        pendingDeepLink = null;
      } else if (parsedUrl.hostname === 'sessions') {
        firstOpenWindow.webContents.send('open-shared-session', pendingDeepLink);
        pendingDeepLink = null;
      }
    } else {
      openUrlHandledLaunch = true;
      firstOpenWindow = await createChat(app, undefined, openDir || undefined);
    }
  }
});

// Handle macOS drag-and-drop onto dock icon
app.on('will-finish-launching', () => {
  if (process.platform === 'darwin') {
    app.setAboutPanelOptions({
      applicationName: 'Biorouter',
      applicationVersion: app.getVersion(),
    });
  }
});

// Handle drag-and-drop onto dock icon
app.on('open-file', async (event, filePath) => {
  event.preventDefault();
  if (isBrxtFile(filePath)) {
    if (app.isReady()) {
      handleBrxtFileOpen(filePath);
    } else {
      app.whenReady().then(() => handleBrxtFileOpen(filePath));
    }
    return;
  }
  await handleFileOpen(filePath);
});

// Handle multiple files/folders (macOS only)
if (process.platform === 'darwin') {
  // Use type assertion for non-standard Electron event
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  app.on('open-files' as any, async (event: any, filePaths: string[]) => {
    event.preventDefault();
    for (const filePath of filePaths) {
      await handleFileOpen(filePath);
    }
  });
}

async function handleFileOpen(filePath: string) {
  try {
    if (!filePath || typeof filePath !== 'string') {
      return;
    }

    const stats = fsSync.lstatSync(filePath);
    let targetDir = filePath;

    // If it's a file, use its parent directory
    if (stats.isFile()) {
      targetDir = path.dirname(filePath);
    }

    // Add to recent directories
    addRecentDir(targetDir);

    // Create new window for the directory
    const newWindow = await createChat(app, undefined, targetDir);

    // Focus the new window
    if (newWindow) {
      newWindow.show();
      newWindow.focus();
      newWindow.moveTop();
    }
  } catch (error) {
    console.error('Failed to handle file open:', error);

    // Show user-friendly error notification
    new Notification({
      title: 'Biorouter',
      body: `Could not open directory: ${path.basename(filePath)}`,
    }).show();
  }
}

declare var MAIN_WINDOW_VITE_DEV_SERVER_URL: string;
declare var MAIN_WINDOW_VITE_NAME: string;

// State for environment variable toggles
let envToggles: EnvToggles = loadSettings().envToggles;

// Parse command line arguments
const parseArgs = () => {
  let dirPath = null;

  // Remove first two elements in dev mode (electron and script path)
  const args = !dirPath && app.isPackaged ? process.argv : process.argv.slice(2);
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--dir' && i + 1 < args.length) {
      dirPath = args[i + 1];
      break;
    }
  }

  return { dirPath };
};

interface BundledConfig {
  defaultProvider?: string;
  defaultModel?: string;
  predefinedModels?: string;
  baseUrlShare?: string;
  version?: string;
}

const getBundledConfig = (): BundledConfig => {
  //{env-macro-start}//
  //needed when biorouter is bundled for a specific provider
  //{env-macro-end}//
  return {
    defaultProvider: process.env.BIOROUTER_DEFAULT_PROVIDER,
    defaultModel: process.env.BIOROUTER_DEFAULT_MODEL,
    predefinedModels: process.env.BIOROUTER_PREDEFINED_MODELS,
    baseUrlShare: process.env.BIOROUTER_BASE_URL_SHARE,
    version: process.env.BIOROUTER_VERSION,
  };
};

const { defaultProvider, defaultModel, predefinedModels, baseUrlShare, version } =
  getBundledConfig();

const GENERATED_SECRET = crypto.randomBytes(32).toString('hex');

const getServerSecret = (settings: ReturnType<typeof loadSettings>): string => {
  if (settings.externalBiorouterd?.enabled && settings.externalBiorouterd.secret) {
    return settings.externalBiorouterd.secret;
  }
  if (process.env.BIOROUTER_EXTERNAL_BACKEND) {
    return 'test';
  }
  return GENERATED_SECRET;
};

/**
 * Issue #56 DR-16: the proof that a request to raise a chat's privacy
 * capability came from the person at the keyboard rather than from the model.
 *
 * 32 random bytes per launch. The RAW key never leaves this process except
 * across the IPC bridge to the renderer, which sends it as `X-User-Action` on
 * the tier-raising calls; the daemon is handed only its SHA-256 digest, on
 * stdin (see `biorouterd.ts`). Not the environment and not argv, because AR-11
 * measured both to be recoverable in-process — which is why open question 23
 * refuses an env-var escape hatch outright.
 *
 * ⚠ What this does NOT close: a caller who can read THIS process, or who can
 * start their own `biorouterd` with a key they chose, is unaffected. Both are
 * the same-machine-caller problem Open question 20 carries; neither is made
 * worse here.
 */
const GENERATED_USER_ACTION_KEY = crypto.randomBytes(32).toString('hex');

/**
 * Deliberately public on the external-backend path: `just debug-server`
 * publishes the digest of this same constant, so `just debug-ui` keeps working.
 * It weakens nothing in the shipped app, whose key is 32 random bytes per
 * launch. See Open question 23.
 */
const DEV_USER_ACTION_KEY = 'biorouter-dev-user-action';

const getUserActionKey = (settings: ReturnType<typeof loadSettings>): string => {
  // A backend the app did not start has whatever user-proof its launcher chose.
  // Absent, this returns '' and every raise fails closed — the right default.
  if (settings.externalBiorouterd?.enabled) {
    return settings.externalBiorouterd.userActionKey ?? '';
  }
  if (process.env.BIOROUTER_EXTERNAL_BACKEND) {
    return DEV_USER_ACTION_KEY;
  }
  return GENERATED_USER_ACTION_KEY;
};

let appConfig = {
  BIOROUTER_DEFAULT_PROVIDER: defaultProvider,
  BIOROUTER_DEFAULT_MODEL: defaultModel,
  BIOROUTER_PREDEFINED_MODELS: predefinedModels,
  BIOROUTER_API_HOST: 'http://127.0.0.1',
  BIOROUTER_WORKING_DIR: '',
  // If BIOROUTER_ALLOWLIST_WARNING env var is not set, defaults to false (strict blocking mode)
  BIOROUTER_ALLOWLIST_WARNING: process.env.BIOROUTER_ALLOWLIST_WARNING === 'true',
};

const windowMap = new Map<number, BrowserWindow>();
const biorouterdClients = new Map<number, Client>();
const managedAppPreviewBackends = new Map<number, ManagedAppPreviewBackend>();

const trackArtifactPreviewFrames = (contents: Electron.WebContents) => {
  const frameIds = new Set<string>();
  contents.on('frame-created', (_event, { frame }) => {
    if (frame?.name === 'biorouter-artifact-preview') {
      frameIds.add(`${frame.processId}:${frame.routingId}`);
    }
  });
  return (frame: Electron.WebFrameMain | null | undefined) => {
    let current = frame;
    while (current) {
      if (
        current.name === 'biorouter-artifact-preview' ||
        frameIds.has(`${current.processId}:${current.routingId}`)
      ) {
        return true;
      }
      current = current.parent;
    }
    return false;
  };
};

// A backend must outlive any single dependent window, so it is ref-counted and
// killed only when the LAST window using it closes. It was written for an
// inherited IPC handler that opened a second window sharing the launcher's
// client: closing the chat window tore down a backend that window was still
// using, and nothing respawned it. That handler is gone and every window now
// retains its own backend, so the count is 1 in practice — the mechanism is kept
// because a release path that assumed sole ownership would be wrong the day a
// shared-backend window returns. `app.on('will-quit')` in biorouterd.ts still
// sweeps every backend on quit, so nothing leaks on exit.
const windowBackends = new Map<number, ChildProcess>(); // windowId -> its backend
const backendRefCounts = new Map<ChildProcess, number>(); // backend -> live windows

const retainBackend = (windowId: number, proc: ChildProcess) => {
  windowBackends.set(windowId, proc);
  backendRefCounts.set(proc, (backendRefCounts.get(proc) ?? 0) + 1);
};

const releaseBackend = (windowId: number) => {
  const proc = windowBackends.get(windowId);
  if (!proc) return;
  windowBackends.delete(windowId);
  const remaining = (backendRefCounts.get(proc) ?? 1) - 1;
  if (remaining > 0) {
    backendRefCounts.set(proc, remaining);
    return; // other windows still depend on this backend
  }
  backendRefCounts.delete(proc);
  if (typeof proc === 'object' && 'kill' in proc) {
    proc.kill(); // last dependent window closed -> safe to terminate
  }
};

// Track power save blockers per window
const windowPowerSaveBlockers = new Map<number, number>(); // windowId -> blockerId
// Track pending initial messages per window
const pendingInitialMessages = new Map<number, string>(); // windowId -> initialMessage

interface ChatWindowOptions {
  initialBounds?: Rectangle;
  show?: boolean;
  manageWindowState?: boolean;
  /** Canonical title for the resumed session, carried in the URL so the tab is
   *  born with the real name (e.g. a diverge branch name) instead of the
   *  "New chat" placeholder it would otherwise show until the session loads. */
  resumeSessionTitle?: string;
}

const createChat = async (
  app: App,
  initialMessage?: string,
  dir?: string,
  _version?: string,
  resumeSessionId?: string,
  viewType?: string,
  workflowDeeplink?: string, // Raw deeplink decoded on server
  scheduledJobId?: string, // Scheduled job ID if applicable
  workflowId?: string,
  workflowParameters?: Record<string, string>, // Workflow parameter values from deeplink URL
  windowOptions?: ChatWindowOptions
) => {
  updateEnvironmentVariables(envToggles);

  const settings = loadSettings();
  const serverSecret = getServerSecret(settings);
  const userActionKey = getUserActionKey(settings);

  // BR-54 Slice A: share ONE daemon across all windows (default). The daemon is
  // already a session-keyed singleton, so its spawn cwd is just a fallback —
  // start it at the home dir and let each window carry its own working directory
  // to its session via REQUEST_DIR / BIOROUTER_WORKING_DIR (`windowWorkingDir`
  // below), which is unchanged. Set BIOROUTER_SHARED_DAEMON=0 to revert to the
  // previous per-window daemon.
  const useSharedDaemon = isSharedDaemonEnabled();
  const windowWorkingDir = path.resolve(path.normalize(dir || os.homedir()));

  const biorouterdResult = useSharedDaemon
    ? await getSharedBackend(startBiorouterd, {
        app,
        serverSecret,
        userActionKey,
        dir: os.homedir(),
        env: { BIOROUTER_PATH_ROOT: process.env.BIOROUTER_PATH_ROOT },
        externalBiorouterd: settings.externalBiorouterd,
      })
    : await startBiorouterd({
        app,
        serverSecret,
        userActionKey,
        dir: dir || os.homedir(),
        env: { BIOROUTER_PATH_ROOT: process.env.BIOROUTER_PATH_ROOT },
        externalBiorouterd: settings.externalBiorouterd,
      });

  const { baseUrl, process: biorouterdProcess, errorLog } = biorouterdResult;
  // Per-window working dir — NOT the shared daemon's spawn cwd. In the
  // per-window (non-shared) path this equals biorouterdResult.workingDir.
  const workingDir = windowWorkingDir;

  const mainWindowState = windowStateKeeper({
    // First-launch size (windowStateKeeper remembers the user's own size after
    // that). Sized so the Home view opens with the usage heatmap AND the recent
    // chats both visible above the composer, rather than the heatmap alone.
    defaultWidth: 1440,
    defaultHeight: 1000,
  });
  const initialBounds = windowOptions?.initialBounds;

  const mainWindow = new BrowserWindow({
    titleBarStyle: process.platform === 'darwin' ? 'hidden' : 'default',
    trafficLightPosition: process.platform === 'darwin' ? { x: 20, y: 16 } : undefined,
    vibrancy: process.platform === 'darwin' ? 'window' : undefined,
    frame: process.platform !== 'darwin',
    x: initialBounds?.x ?? mainWindowState.x,
    y: initialBounds?.y ?? mainWindowState.y,
    width: initialBounds?.width ?? mainWindowState.width,
    height: initialBounds?.height ?? mainWindowState.height,
    // DERIVED, not chosen: the 288px sidebar DEFAULT (`SIDEBAR_DEFAULT_WIDTH`
    // in components/ui/sidebarWidth.ts) + the 760px reading column
    // (`--measure-chat` in styles/main.css) = 1048. `useContentSize` is on, so
    // this is content width, which is the number the renderer sees.
    //
    // ⚠ The sidebar's DEFAULT, not its minimum. The sidebar is user-resizable
    // (216–360px, default 288), and it is tempting to take the floor from the
    // bottom of that range on the argument that the minimum is the only width
    // that is a property of the app rather than of a preference. That argument
    // gets the direction backwards: a floor of 216 + 760 = 976 is a promise
    // about a width NO install has until someone drags the edge, and at the
    // width every install actually ships with it leaves the column 976 − 288 =
    // 688px — under the very measure this floor exists to protect. The default
    // is the sidebar the window must be able to seat.
    //
    // Past the default the user is giving up reading room deliberately, with
    // the window already open and the edge under their hand, and can give it
    // back the same way; a floor cannot promise anything about a preference it
    // is never told. The wide end is bounded separately and by construction:
    // 360 + 760 = 1120 = SIDEBAR_COMPACT_WIDTH, so raising SIDEBAR_MAX_WIDTH
    // past the point where rung 1 of the yield ladder collapses the sidebar to
    // an overlay would start eating the measure. `styles/measures.test.ts` pins
    // both the floor here and that identity.
    //
    // Below it the Home column is narrower than its own measure, and the usage
    // heatmap — the one thing on Home whose size is computed rather than
    // declared — starts shrinking its cells. Measured in a browser against the
    // real stylesheet, WITH THE THEN-240px SIDEBAR: at 990px content width the
    // grid is still at its full 23px cells; at 980px it drops to 22px and keeps
    // stepping down to 16px by 800px, with the exact cliff at 989px.
    //
    // ⚠ That 989 is a WINDOW width and therefore carries the sidebar of the day
    // inside it — what the heatmap actually reacts to is its own column, i.e.
    // window minus sidebar, so the cliff moves with the sidebar. The measured
    // cliff is a 749px column (989 − 240); against the 288px default the same
    // cliff is a 1037px window, and 1048 clears it by the same 11px the old 1000
    // cleared 989 by. The floor is still the tokens rather than the measurement,
    // so it survives the heatmap's cell ladder changing.
    // `styles/measures.test.ts` asserts the arithmetic still holds.
    //
    // ⚠ This is a *minimum window size*, not a content `max-width` — it is not
    // the flat pixel cap that docs/desktop-ui/window-scaling-regressions.md
    // warns about. It puts a floor under the window; it does nothing to a wide
    // one, where `--measure-page` still tracks the pane as a percentage.
    //
    // The height axis is deliberately NOT capped to the same standard. A short
    // window shrinks the heatmap's cells too, but the minHeight that would stop
    // it lands around 700-800px, which is unusable on a 1280x800 display once
    // the menu bar and Dock are taken out. The heatmap keeps its chrome locked
    // to its grid instead (see UsageHeatmap's `heatStyle`), so a squeezed grid
    // stays a coherent block rather than desyncing from its own labels.
    minWidth: 1048,
    minHeight: 600,
    resizable: true,
    useContentSize: true,
    show: windowOptions?.show ?? true,
    icon: resolveImagePath(
      process.platform === 'win32'
        ? 'icon.ico'
        : process.platform === 'darwin'
          ? 'icon.icns'
          : 'icon.png'
    ),
    webPreferences: {
      spellcheck: settings.spellcheckEnabled ?? true,
      preload: path.join(__dirname, 'preload.js'),
      webSecurity: true,
      // Throttle timers/rAF/reconciliation in backgrounded windows. This is
      // Electron's default; set explicitly so a future window-pooling change
      // can't silently lose it (each project window is a full renderer process).
      backgroundThrottling: true,
      nodeIntegration: false,
      contextIsolation: true,
      additionalArguments: [
        JSON.stringify({
          ...appConfig,
          BIOROUTER_API_HOST: baseUrl,
          BIOROUTER_WORKING_DIR: workingDir,
          REQUEST_DIR: dir,
          BIOROUTER_BASE_URL_SHARE: baseUrlShare,
          BIOROUTER_VERSION: version,
          workflowId: workflowId,
          workflowDeeplink: workflowDeeplink,
          workflowParameters: workflowParameters,
          scheduledJobId: scheduledJobId,
        }),
      ],
      partition: 'persist:biorouter',
    },
  });

  if (!app.isPackaged) {
    installExtension(REACT_DEVELOPER_TOOLS, {
      loadExtensionOptions: { allowFileAccess: true },
      session: mainWindow.webContents.session,
    })
      .then(() => log.info('added react dev tools'))
      .catch((err) => log.info('failed to install react dev tools:', err));
  }

  const biorouterdClient = createClient(
    createConfig({
      baseUrl,
      headers: {
        'Content-Type': 'application/json',
        'X-Secret-Key': serverSecret,
      },
    })
  );
  biorouterdClients.set(mainWindow.id, biorouterdClient);
  const managedPreviewBackend = bindManagedAppPreviewBackend(biorouterdResult, mainWindow);
  if (managedPreviewBackend) managedAppPreviewBackends.set(mainWindow.id, managedPreviewBackend);
  // With a shared daemon the backend is app-lifetime (killed only in
  // startBiorouterd's own `will-quit` sweep), so windows must NOT ref-count it —
  // closing one window must not tear the daemon out from under the others. The
  // per-window ref-count is only for the (opt-out) per-window daemon path.
  if (!useSharedDaemon) {
    retainBackend(mainWindow.id, biorouterdProcess);
  }

  const serverReady = await checkServerStatus(biorouterdClient, errorLog);
  if (!serverReady) {
    const isUsingExternalBackend = settings.externalBiorouterd?.enabled;

    if (isUsingExternalBackend) {
      const response = dialog.showMessageBoxSync({
        type: 'error',
        title: 'External backend unreachable',
        message: `Could not connect to external backend at ${settings.externalBiorouterd?.url}`,
        detail: 'The external biorouterd server may not be running.',
        buttons: ['Disable External Backend & Retry', 'Quit'],
        defaultId: 0,
        cancelId: 1,
      });

      if (response === 0) {
        const updatedSettings = {
          ...settings,
          externalBiorouterd: {
            enabled: false,
            url: settings.externalBiorouterd?.url || '',
            secret: settings.externalBiorouterd?.secret || '',
          },
        };
        saveSettings(updatedSettings);
        // The shared daemon was started against the now-disabled external
        // config; forget it so the retry starts a fresh local daemon.
        resetSharedBackend();
        mainWindow.destroy();
        return createChat(app, initialMessage, dir);
      }
    } else {
      dialog.showMessageBoxSync({
        type: 'error',
        title: 'Biorouter failed to start',
        message: 'The backend server failed to start.',
        detail: errorLog.join('\n'),
        buttons: ['OK'],
      });
    }
    app.quit();
  }

  if (windowOptions?.manageWindowState !== false) {
    mainWindowState.manage(mainWindow);
  }

  mainWindow.webContents.session.setSpellCheckerLanguages(['en-US', 'en-GB']);
  mainWindow.webContents.on('context-menu', (_event, params) => {
    const menu = new Menu();
    const hasSpellingSuggestions = params.dictionarySuggestions.length > 0 || params.misspelledWord;

    if (hasSpellingSuggestions) {
      for (const suggestion of params.dictionarySuggestions) {
        menu.append(
          new MenuItem({
            label: suggestion,
            click: () => mainWindow.webContents.replaceMisspelling(suggestion),
          })
        );
      }

      if (params.misspelledWord) {
        menu.append(
          new MenuItem({
            label: 'Add to dictionary',
            click: () =>
              mainWindow.webContents.session.addWordToSpellCheckerDictionary(params.misspelledWord),
          })
        );
      }

      if (params.selectionText) {
        menu.append(new MenuItem({ type: 'separator' }));
      }
    }
    if (params.selectionText) {
      menu.append(
        new MenuItem({
          label: 'Cut',
          accelerator: 'CmdOrCtrl+X',
          role: 'cut',
        })
      );
      menu.append(
        new MenuItem({
          label: 'Copy',
          accelerator: 'CmdOrCtrl+C',
          role: 'copy',
        })
      );
    }

    // Only show paste in editable fields (text inputs)
    if (params.isEditable) {
      menu.append(
        new MenuItem({
          label: 'Paste',
          accelerator: 'CmdOrCtrl+V',
          role: 'paste',
        })
      );
    }

    if (menu.items.length > 0) {
      menu.popup();
    }
  });

  // Handle new window creation for links.
  //
  // Deny by default. An `allow` here would open a BrowserWindow that inherits
  // this window's webPreferences -- including the preload IPC bridge -- and
  // non-http(s) schemes (`data:`, `blob:`, `about:`) receive no CSP, since the
  // CSP is injected by onHeadersReceived. Agent-authored artifact HTML must
  // never reach such a window.
  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (shouldOpenExternalNavigation(url, rendererEntryUrl())) {
      void openExternalBrowserNavigation(mainWindow, url);
    }
    return { action: 'deny' };
  });

  // Handle new-window events (alternative approach for external links)
  // Use type assertion for non-standard Electron event
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  mainWindow.webContents.on('new-window' as any, function (event: any, url: string) {
    event.preventDefault();
    // Unlike setWindowOpenHandler above, this legacy path used to hand any
    // scheme -- including file:// and custom protocols -- to the OS opener.
    if (shouldOpenExternalNavigation(url, rendererEntryUrl())) {
      void openExternalBrowserNavigation(mainWindow, url);
    }
  });

  // Nothing in this app navigates the top frame away from its own origin. A
  // file:// or data: navigation would keep the preload bridge and get no CSP.
  const blockOffOriginNavigation = (event: Electron.Event, url: string) => {
    if (isAppOrigin(url, rendererEntryUrl())) return;
    log.warn('[Main] Blocked off-origin navigation to', url);
    event.preventDefault();
    if (isExternallyOpenableUrl(url)) {
      void openExternalBrowserNavigation(mainWindow, url);
    }
  };
  mainWindow.webContents.on('will-navigate', blockOffOriginNavigation);
  const isArtifactPreviewFrame = trackArtifactPreviewFrames(mainWindow.webContents);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  mainWindow.webContents.on('will-frame-navigate' as any, (event: any) => {
    if (event.isMainFrame) {
      blockOffOriginNavigation(event, event.url);
      return;
    }
    if (isArtifactPreviewFrame(event.frame) && !isAllowedArtifactFrameNavigation(event.url)) {
      log.warn('[Main] Blocked artifact frame navigation to', event.url);
      event.preventDefault();
    }
  });

  const windowId = mainWindow.id;
  const url = MAIN_WINDOW_VITE_DEV_SERVER_URL
    ? new URL(MAIN_WINDOW_VITE_DEV_SERVER_URL)
    : pathToFileURL(path.join(__dirname, `../renderer/${MAIN_WINDOW_VITE_NAME}/index.html`));

  let appPath = '/';
  const routeMap: Record<string, string> = {
    chat: '/',
    pair: '/pair',
    settings: '/settings',
    sessions: '/sessions',
    schedules: '/schedules',
    workflows: '/workflows',
    permission: '/permission',
    ConfigureProviders: '/configure-providers',
    sharedSession: '/shared-session',
    welcome: '/welcome',
  };

  if (viewType) {
    appPath = routeMap[viewType] || '/';
  }
  if (
    appPath === '/' &&
    (workflowDeeplink !== undefined || workflowId !== undefined || initialMessage)
  ) {
    appPath = '/pair';
  }

  let searchParams = new URLSearchParams();
  if (resumeSessionId) {
    searchParams.set('resumeSessionId', resumeSessionId);
    // A fresh window has no react-router location.state, so the tab title must
    // ride the URL. useChatGroupsUrlSync reads it as the opening tab's title.
    if (windowOptions?.resumeSessionTitle) {
      searchParams.set('resumeSessionTitle', windowOptions.resumeSessionTitle);
    }
    if (appPath === '/') {
      appPath = '/pair';
    }
  }
  // Only add workflowId to URL for the non-deeplink case (saved workflows launched from UI)
  // For deeplinks, the workflow object is passed via appConfig, not URL params
  if (workflowId) {
    searchParams.set('workflowId', workflowId);
    if (appPath === '/') {
      appPath = '/pair';
    }
  }

  // The initial message CANNOT ride the URL: it is delivered as IPC only after
  // the renderer signals react-ready (pendingInitialMessages below). Until that
  // round trip completes the window sits on a bare /pair with zero tabs and no
  // route state, which the empty-pair redirect (issue #38) would read as a
  // stale deep link and bounce Home before the cargo ever arrives. This marker
  // is the synchronous bootstrap flag the redirect can see from the very first
  // render; it disappears when App.tsx's set-initial-message handler navigates
  // to /pair with the real route state.
  if (initialMessage) {
    searchParams.set('initialMessagePending', 'true');
  }

  // Biorouter's react app uses HashRouter, so the path + search params follow a #/
  url.hash = `${appPath}?${searchParams.toString()}`;
  let formattedUrl = formatUrl(url);
  log.info('Opening URL: ', formattedUrl);
  mainWindow.loadURL(formattedUrl);

  // If we have an initial message, store it to send after React is ready
  if (initialMessage) {
    pendingInitialMessages.set(mainWindow.id, initialMessage);
  }

  // Set up local keyboard shortcuts that only work when the window is focused
  mainWindow.webContents.on('before-input-event', (event, input) => {
    if (input.key === 'r' && input.meta) {
      mainWindow.reload();
      event.preventDefault();
    }

    if (input.key === 'i' && input.alt && input.meta) {
      mainWindow.webContents.openDevTools();
      event.preventDefault();
    }
  });

  mainWindow.on('app-command', (e, cmd) => {
    if (cmd === 'browser-backward') {
      mainWindow.webContents.send('mouse-back-button-clicked');
      e.preventDefault();
    }
  });

  // Handle mouse back button (button 3)
  // Use type assertion for non-standard Electron event
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  mainWindow.webContents.on('mouse-up' as any, function (_event: any, mouseButton: number) {
    // MouseButton 3 is the back button.
    if (mouseButton === 3) {
      mainWindow.webContents.send('mouse-back-button-clicked');
    }
  });

  windowMap.set(windowId, mainWindow);
  // Recorded here, from main's own `windowWorkingDir`, so the preview allowlist
  // can admit the folder the user chose for THIS chat without ever trusting a
  // renderer to name it.
  windowWorkingDirs.set(windowId, workingDir);

  // ── Tab tear-off: keep the strip-band registry's view of this window honest.
  //
  // Z-ORDER HAS NO SOURCE IN ELECTRON (design D3). There is no z-order query and
  // `getAllWindows()` order is not documented as one, so focus recency stands in
  // for it — which only works if something actually raises. Without these five
  // listeners the "topmost window wins" rule silently degrades to REGISTRATION
  // order, and is wrong the first time a user clicks between two windows.
  //
  // `hidden` is the other half: a minimised or hidden window is neither a merge
  // target nor an OCCLUDER, so it must not shadow a visible window behind it.
  // Window MOVES need no listener here — see refreshRegisteredContentBounds.
  mainWindow.on('focus', () => stripBandRegistry.raise(windowId));
  mainWindow.on('show', () => {
    stripBandRegistry.setHidden(windowId, false);
    stripBandRegistry.raise(windowId);
  });
  mainWindow.on('restore', () => {
    stripBandRegistry.setHidden(windowId, false);
    stripBandRegistry.raise(windowId);
  });
  mainWindow.on('hide', () => stripBandRegistry.setHidden(windowId, true));
  mainWindow.on('minimize', () => stripBandRegistry.setHidden(windowId, true));

  // `closed` is not enough on its own: a reload or a renderer crash keeps the
  // window and kills the drag. See registerTabDragOwnerTeardown.
  registerTabDragOwnerTeardown(mainWindow, windowId);
  registerEmbeddedBrowserOwnerTeardown(mainWindow);

  // Handle window closure
  mainWindow.on('closed', () => {
    windowMap.delete(windowId);
    managedAppPreviewBackends.delete(windowId);
    windowWorkingDirs.delete(windowId);
    // A closed window stops being a merge target on the very next pointermove
    // (design §6), and cannot be left holding a caret nobody will clear.
    forgetWindowFromTabDrag(windowId, 'window closed');

    // Embedded browser views are children of this window's contentView, not of
    // the React tree, so nothing in the renderer unmounts them.
    destroyEmbeddedBrowsersForWindow(mainWindow);

    // Clean up pending initial message
    pendingInitialMessages.delete(windowId);

    if (windowPowerSaveBlockers.has(windowId)) {
      const blockerId = windowPowerSaveBlockers.get(windowId)!;
      try {
        powerSaveBlocker.stop(blockerId);
        console.log(
          `[Main] Stopped power save blocker ${blockerId} for closing window ${windowId}`
        );
      } catch (error) {
        console.error(
          `[Main] Failed to stop power save blocker ${blockerId} for window ${windowId}:`,
          error
        );
      }
      windowPowerSaveBlockers.delete(windowId);
    }

    // Kill this window's backend only if no other window still shares it.
    releaseBackend(windowId);
  });
  return mainWindow;
};

/**
 * Open a diverged (branched) session in a NEW, focused window, leaving every
 * existing window exactly in place. Backs the `biorouter://diverge` deeplink
 * (CLI/TUI `/diverge`) and mirrors the in-app Diverge button. The new window is
 * offset from the focused one so the user can see it's a distinct, second
 * window rather than a silent in-place clone.
 */
async function openDivergedWindow(parsedUrl: URL): Promise<void> {
  const sessionId = parsedUrl.searchParams.get('session_id') || undefined;
  if (!sessionId) {
    log.error('[Main] diverge deeplink missing session_id:', parsedUrl.toString());
    return;
  }
  const recentDirs = loadRecentDirs();
  const dir =
    parsedUrl.searchParams.get('dir') ||
    (recentDirs.length > 0 ? recentDirs[0] : undefined) ||
    undefined;

  await openDivergedChatWindow(sessionId, dir);
}

function branchWindowBounds(anchor?: BrowserWindow | null): Rectangle | undefined {
  if (!anchor || anchor.isDestroyed()) return undefined;

  const anchorBounds = anchor.getBounds();
  const display = screen.getDisplayMatching(anchorBounds);
  const workArea = display.workArea;
  const width = Math.min(anchorBounds.width, workArea.width);
  const height = Math.min(anchorBounds.height, workArea.height);
  let x = anchorBounds.x + 40;
  let y = anchorBounds.y + 40;

  if (x + width > workArea.x + workArea.width) {
    x = Math.max(workArea.x, anchorBounds.x - 40);
  }
  if (y + height > workArea.y + workArea.height) {
    y = Math.max(workArea.y, anchorBounds.y - 40);
  }

  return { x, y, width, height };
}

async function openDivergedChatWindow(
  sessionId: string,
  dir?: string,
  sourceWindow?: BrowserWindow | null,
  title?: string
): Promise<void> {
  const anchor = BrowserWindow.getFocusedWindow() ?? BrowserWindow.getAllWindows()[0];
  const bounds = branchWindowBounds(sourceWindow ?? anchor);
  const win = await createChat(
    app,
    undefined,
    dir,
    undefined,
    sessionId,
    'pair',
    undefined,
    undefined,
    undefined,
    undefined,
    {
      ...(bounds ? { initialBounds: bounds, show: false, manageWindowState: false } : {}),
      ...(title ? { resumeSessionTitle: title } : {}),
    }
  );
  if (win) {
    win.show();
    win.focus();
    win.moveTop();
  }
}

// ═══════════════════════════════════════════════════════════════════════════
// TAB TEAR-OFF AND MERGE — the main-process half of the gesture (Phase 3).
//
// docs/design/astryx-adoption/tab-tear-off-and-merge.md. The pure geometry is
// `windowDrag.ts`; everything here is the Electron it needs plus the ownership
// rules it cannot express. Main is in this at all for one reason: while the
// mouse button is held, the OS delivers every pointer event to the SOURCE
// window. The window being dragged over receives nothing (measured in Phase 0
// with real OS input), so it cannot hit-test itself, and `getBounds()` has no
// renderer equivalent. Main is the only party that can answer "which window is
// under this point".
// ═══════════════════════════════════════════════════════════════════════════

const stripBandRegistry = new StripBandRegistry();

/** Built lazily: Electron's `screen` module is only usable after `app` is ready. */
let cachedScreenGeometry: ScreenGeometry | null = null;
function tabDragGeometry(): ScreenGeometry {
  if (!cachedScreenGeometry) cachedScreenGeometry = electronScreenGeometry(screen);
  return cachedScreenGeometry;
}

/**
 * Who is showing a merge caret, who is holding the pointer, and which merges are
 * still unanswered. The rules live in `windowDrag.ts` where they can be tested;
 * this supplies the Electron they need and nothing else.
 */
const tabDragBroker = new TabDragBroker({
  resolveWindow: (windowId) => {
    const win = windowMap.get(windowId);
    if (!win) return null;
    return {
      webContentsId: win.isDestroyed() ? -1 : win.webContents.id,
      isAlive: () => !win.isDestroyed(),
      send: (channel, payload) => {
        if (!win.isDestroyed()) win.webContents.send(channel, payload);
      },
    };
  },
});

/**
 * THE GHOST THAT FOLLOWS THE CURSOR ONTO THE DESKTOP (issue #75, design Phase 4b).
 *
 * The rules — placement, markup, lifecycle — are in `dragGhostWindow.ts`, which
 * imports no Electron so they can be tested. This is only the Electron they
 * need: read the source window's DOM ghost, build one window, tell the source
 * renderer to hide its own `<div>` ghost while ours is up.
 *
 * THE WINDOW RECIPE IS THE LAUNCHER'S, MINUS THE PARTS THAT WOULD END THE
 * GESTURE. `createLauncher` below is already frameless, darwin-transparent,
 * always-on-top and off the taskbar, which is most of what is wanted. Three
 * differences, and each is load-bearing:
 *
 *   - `focusable: false` + `setIgnoreMouseEvents(true)` + `showInactive()`. The
 *     source window holds the pointer capture that IS the drag; raising or
 *     focusing any window drops it and the gesture dies in mid-air. The launcher
 *     wants focus (it destroys itself on blur); this must never take it.
 *   - NO `vibrancy`. The design flags it as a hazard, and a 30px chip does not
 *     want a blurred backdrop of whatever it is flying over.
 *   - `hasShadow: false`. The window is bigger than the chip (transparent slack
 *     for the outline and the CSS shadow), so a NATIVE shadow would trace that
 *     larger rectangle and draw a box around a ghost that has no box.
 */
async function createDragGhostWindow(
  bounds: DragRect,
  spec: GhostSpec
): Promise<GhostWindowHandle | null> {
  const transparent = process.platform === 'darwin';
  const ghost = new BrowserWindow({
    x: bounds.x,
    y: bounds.y,
    width: bounds.width,
    height: bounds.height,
    show: false,
    frame: false,
    transparent,
    backgroundColor: transparent ? '#00000000' : spec.style.background,
    hasShadow: false,
    focusable: false,
    skipTaskbar: true,
    alwaysOnTop: true,
    resizable: false,
    movable: false,
    minimizable: false,
    maximizable: false,
    fullscreenable: false,
    acceptFirstMouse: false,
    webPreferences: {
      // NO PRELOAD, no node, no app bundle. The page is a static chip; giving it
      // the app's preload would put every IPC door this process exposes behind a
      // window created inside a pointer gesture, for a document that has nothing
      // to say.
      nodeIntegration: false,
      contextIsolation: true,
      sandbox: true,
      devTools: false,
      spellcheck: false,
    },
  });
  // Click-through: the cursor is mid-drag and every event belongs to the source
  // window. A ghost that swallowed one would end the gesture under itself.
  ghost.setIgnoreMouseEvents(true);
  // Above the source window AND above other apps' windows — the ghost is drawn
  // over the desktop the user is dragging across, not over ours alone.
  ghost.setAlwaysOnTop(true, 'screen-saver');

  // Attached BEFORE the load: `ready-to-show` can fire while `loadURL`'s promise
  // is still settling, and a listener added afterwards would wait for an event
  // that has already gone by until the timeout rescued it.
  const painted = new Promise<void>((resolve) => {
    ghost.once('ready-to-show', () => resolve());
    setTimeout(resolve, 250);
  });
  try {
    await ghost.loadURL(ghostWindowDataUrl(ghostWindowHtml(spec, { transparent })));
    await painted;
  } catch (error) {
    log.warn('[tab-drag] ghost window failed to load:', error);
    if (!ghost.isDestroyed()) ghost.destroy();
    return null;
  }
  if (ghost.isDestroyed()) return null;

  return {
    setPosition: (x, y) => {
      if (!ghost.isDestroyed()) ghost.setPosition(x, y, false);
    },
    // showInactive, NEVER show(): `show()` activates, and activation is exactly
    // the focus theft that ends the drag.
    show: () => {
      if (!ghost.isDestroyed()) ghost.showInactive();
    },
    destroy: () => {
      if (!ghost.isDestroyed()) ghost.destroy();
    },
  };
}

const dragGhostWindows = new DragGhostWindowController(
  {
    probeSource: async (sourceWindowId) => {
      const win = windowMap.get(sourceWindowId);
      if (!win || win.isDestroyed()) return null;
      try {
        return await win.webContents.executeJavaScript(GHOST_PROBE_SCRIPT, false);
      } catch (error) {
        // A reloading or dying renderer. The controller falls back to defaults
        // rather than showing nothing.
        log.warn('[tab-drag] ghost probe failed:', error);
        return null;
      }
    },
    createWindow: (_sourceWindowId, bounds, spec) => createDragGhostWindow(bounds, spec),
    notifySource: (sourceWindowId, active) => {
      const win = windowMap.get(sourceWindowId);
      if (!win || win.isDestroyed()) return;
      win.webContents.send('tab-drag:ghost-window', { active });
    },
    onError: (message, error) => log.warn(`[tab-drag] ${message}:`, error),
  },
  { inset: process.platform === 'darwin' ? GHOST_TRANSPARENT_INSET : GHOST_OPAQUE_INSET }
);

/**
 * `tab-drag:commit`'s payload as it arrives — every field still unproven.
 *
 * `point` and `grabOffset` are `unknown` rather than their wire shapes on
 * purpose: they are geometry, and geometry only enters this process through
 * `screenPointFromWire`/`grabOffsetFromWire`. Typing them here would let a
 * future edit reach past the converters again.
 */
interface TabDragCommitRequestWire {
  point?: unknown;
  grabOffset?: unknown;
  tab?: {
    sessionId?: string;
    title?: string;
    userSetName?: boolean;
    cwd?: string;
    workflowId?: string;
  };
  isOnlyTab?: boolean;
}

/** A renderer-supplied band list, made safe to store. */
function sanitizeStripBands(input: unknown): DragRect[] {
  if (!Array.isArray(input)) return [];
  const bands: DragRect[] = [];
  // A split window has at most MAX_GROUPS strips; the cap is a bound on what a
  // compromised renderer can make main hold, not a layout rule.
  for (const raw of input.slice(0, 16)) {
    const band = raw as Partial<DragRect>;
    if (
      typeof band?.x !== 'number' ||
      typeof band?.y !== 'number' ||
      typeof band?.width !== 'number' ||
      typeof band?.height !== 'number'
    ) {
      continue;
    }
    if (![band.x, band.y, band.width, band.height].every(Number.isFinite)) continue;
    bands.push({ x: band.x, y: band.y, width: band.width, height: band.height });
  }
  return bands;
}

/**
 * Re-read every registered window's content rect from the live `BrowserWindow`.
 *
 * Bands are viewport-relative and change only when the layout does, so the
 * renderer re-registers them itself. The CONTENT RECT is different: a window
 * dragged to a new position by its title bar fires no renderer resize at all, so
 * a stored rect goes stale with nothing to invalidate it — and a stale rect
 * means a merge resolved against where a window USED to be. Refreshing on every
 * move costs one `getContentBounds()` per window per pointermove and removes the
 * whole class of staleness, which no amount of renderer-side reporting could.
 */
function refreshRegisteredContentBounds(): void {
  for (const [windowId, win] of windowMap) {
    const entry = stripBandRegistry.get(windowId);
    if (!entry) continue;
    if (win.isDestroyed()) {
      stripBandRegistry.remove(windowId);
      continue;
    }
    // `register` preserves stackOrder and hidden — see its doc comment. This is
    // a measurement update, not a raise.
    stripBandRegistry.register(windowId, {
      contentBounds: win.getContentBounds(),
      bands: entry.bands,
    });
  }
}

/**
 * A window has stopped being able to take part in a drag — it closed, its render
 * process died, or a reload replaced its document.
 *
 * ALL THREE MATTER, and only the first was handled. `tab-drag:end` is sent by
 * the SOURCE renderer's unmount cleanup, and a document swap runs no React
 * effect cleanups at all — so a source that was reloaded (Cmd+R is wired to
 * `mainWindow.reload()` in `createChat`) or crashed never sends it, and the
 * target window it had pointed at kept painting an insertion caret forever. This
 * is the same failure `registerTerminalOwnerTeardown` exists for, and it is
 * fixed the same way.
 */
function forgetWindowFromTabDrag(windowId: number, reason: string): void {
  stripBandRegistry.remove(windowId);
  const wasInDrag =
    tabDragBroker.previewWindow === windowId ||
    tabDragBroker.dragSourceWindow === windowId ||
    tabDragBroker.pendingMergeCount > 0 ||
    dragGhostWindows.sourceWindow === windowId;
  tabDragBroker.forgetWindow(windowId);
  // A ghost window outlives nothing. Its source is the only thing that would
  // ever have told it to go away, so a source that crashed, reloaded or closed
  // would otherwise strand a click-through chip on top of every other app, with
  // no gesture behind it and no way for the user to dismiss it.
  dragGhostWindows.releaseIfSource(windowId, reason);
  if (wasInDrag) log.info(`[tab-drag] released window ${windowId}:`, reason);
}

/**
 * Free a chat window from any in-flight drag once its document goes away.
 *
 * Mirrors `registerTerminalOwnerTeardown` line for line, including the two
 * filters that were each learned from a real bug there: `isSameDocument`,
 * because the app is a hash router and `#/pair -> #/settings` fires this event
 * too; and `isAppOrigin`, because `did-start-navigation` fires before the
 * navigation throttles that cancel an off-origin navigation, so a navigation
 * that never commits would otherwise tear the drag down.
 */
function registerTabDragOwnerTeardown(win: BrowserWindow, windowId: number): void {
  win.webContents.on('did-start-navigation', (details) => {
    if (!details.isMainFrame || details.isSameDocument) return;
    if (!isAppOrigin(details.url, rendererEntryUrl())) return;
    forgetWindowFromTabDrag(windowId, 'document replaced');
  });
  win.webContents.on('render-process-gone', () =>
    forgetWindowFromTabDrag(windowId, 'render process gone')
  );
}

const createLauncher = () => {
  const launcherWindow = new BrowserWindow({
    width: 600,
    height: 80,
    frame: false,
    transparent: process.platform === 'darwin',
    backgroundColor: process.platform === 'darwin' ? '#00000000' : '#ffffff',
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      nodeIntegration: false,
      contextIsolation: true,
      additionalArguments: [JSON.stringify(appConfig)],
      partition: 'persist:biorouter',
    },
    skipTaskbar: true,
    alwaysOnTop: true,
    resizable: false,
    movable: true,
    minimizable: false,
    maximizable: false,
    fullscreenable: false,
    hasShadow: true,
    vibrancy: process.platform === 'darwin' ? 'window' : undefined,
  });

  // Center on screen
  const primaryDisplay = screen.getPrimaryDisplay();
  const { width, height } = primaryDisplay.workAreaSize;
  const windowBounds = launcherWindow.getBounds();

  launcherWindow.setPosition(
    Math.round(width / 2 - windowBounds.width / 2),
    Math.round(height / 3 - windowBounds.height / 2)
  );

  // Load launcher window content
  const url = MAIN_WINDOW_VITE_DEV_SERVER_URL
    ? new URL(MAIN_WINDOW_VITE_DEV_SERVER_URL)
    : pathToFileURL(path.join(__dirname, `../renderer/${MAIN_WINDOW_VITE_NAME}/index.html`));

  url.hash = '/launcher';
  launcherWindow.loadURL(formatUrl(url));

  // Destroy window when it loses focus
  launcherWindow.on('blur', () => {
    launcherWindow.destroy();
  });

  // Also destroy on escape key
  launcherWindow.webContents.on('before-input-event', (event, input) => {
    if (input.key === 'Escape') {
      launcherWindow.destroy();
      event.preventDefault();
    }
  });

  return launcherWindow;
};

// Track tray instance
let tray: Tray | null = null;

const destroyTray = () => {
  if (tray) {
    tray.destroy();
    tray = null;
  }
};

const disableTray = () => {
  const settings = loadSettings();
  settings.showMenuBarIcon = false;
  saveSettings(settings);
};

const createTray = () => {
  destroyTray();

  const possiblePaths = [
    path.join(process.resourcesPath, 'images', 'iconTemplate.png'),
    path.join(process.cwd(), 'src', 'images', 'iconTemplate.png'),
    path.join(__dirname, '..', 'images', 'iconTemplate.png'),
    path.join(__dirname, 'images', 'iconTemplate.png'),
    path.join(process.cwd(), 'images', 'iconTemplate.png'),
  ];

  const iconPath = possiblePaths.find((p) => fsSync.existsSync(p));

  if (!iconPath) {
    console.warn('[Main] Tray icon not found. App will continue without system tray.');
    disableTray();
    return;
  }

  try {
    tray = new Tray(iconPath);
    setTrayRef(tray);
    updateTrayMenu(getUpdateAvailable());

    if (process.platform === 'win32' || process.platform === 'darwin') {
      tray.on('click', showWindow);
    }
    if (process.platform === 'darwin') {
      tray.on('right-click', () => {
        popUpTrayMenu();
      });
    }
  } catch (error) {
    console.error('[Main] Tray creation failed. App will continue without system tray.', error);
    disableTray();
    tray = null;
  }
};

const showWindow = async () => {
  const windows = BrowserWindow.getAllWindows();

  if (windows.length === 0) {
    log.info('No windows are open, creating a new one...');
    const recentDirs = loadRecentDirs();
    const openDir = recentDirs.length > 0 ? recentDirs[0] : null;
    await createChat(app, undefined, openDir || undefined);
    return;
  }

  const initialOffsetX = 30;
  const initialOffsetY = 30;

  // Iterate over all windows
  windows.forEach((win, index) => {
    const currentBounds = win.getBounds();
    const newX = currentBounds.x + initialOffsetX * index;
    const newY = currentBounds.y + initialOffsetY * index;

    win.setBounds({
      x: newX,
      y: newY,
      width: currentBounds.width,
      height: currentBounds.height,
    });

    if (!win.isVisible()) {
      win.show();
    }

    win.focus();
  });
};

const buildRecentFilesMenu = () => {
  const recentDirs = loadRecentDirs();
  return recentDirs.map((dir) => ({
    label: dir,
    click: () => {
      createChat(app, undefined, dir);
    },
  }));
};

const openDirectoryDialog = async (): Promise<OpenDialogReturnValue> => {
  // Get the current working directory from the focused window
  let defaultPath: string | undefined;
  const currentWindow = BrowserWindow.getFocusedWindow();

  if (currentWindow) {
    try {
      const currentWorkingDir = await currentWindow.webContents.executeJavaScript(
        `window.appConfig ? window.appConfig.get('BIOROUTER_WORKING_DIR') : null`
      );

      if (currentWorkingDir && typeof currentWorkingDir === 'string') {
        // Verify the directory exists before using it as default
        try {
          const stats = fsSync.lstatSync(currentWorkingDir);
          if (stats.isDirectory()) {
            defaultPath = currentWorkingDir;
          }
        } catch (error) {
          if (error && typeof error === 'object' && 'code' in error) {
            const fsError = error as { code?: string; message?: string };
            if (
              fsError.code === 'ENOENT' ||
              fsError.code === 'EACCES' ||
              fsError.code === 'EPERM'
            ) {
              console.warn(
                `Current working directory not accessible (${fsError.code}): ${currentWorkingDir}, falling back to home directory`
              );
              defaultPath = os.homedir();
            } else {
              console.warn(
                `Unexpected filesystem error (${fsError.code}) for directory ${currentWorkingDir}:`,
                fsError.message
              );
              defaultPath = os.homedir();
            }
          } else {
            console.warn(`Unexpected error checking directory ${currentWorkingDir}:`, error);
            defaultPath = os.homedir();
          }
        }
      }
    } catch (error) {
      console.warn('Failed to get current working directory from window:', error);
    }
  }

  if (!defaultPath) {
    defaultPath = os.homedir();
  }

  const result = (await dialog.showOpenDialog({
    properties: ['openFile', 'openDirectory', 'createDirectory'],
    defaultPath: defaultPath,
  })) as unknown as OpenDialogReturnValue;

  if (!result.canceled && result.filePaths.length > 0) {
    const selectedPath = result.filePaths[0];

    // If a file was selected, use its parent directory
    let dirToAdd = selectedPath;
    try {
      const stats = fsSync.lstatSync(selectedPath);

      // Reject symlinks for security
      if (stats.isSymbolicLink()) {
        console.warn(`Selected path is a symlink, using parent directory for security`);
        dirToAdd = path.dirname(selectedPath);
      } else if (stats.isFile()) {
        dirToAdd = path.dirname(selectedPath);
      }
    } catch {
      console.warn(`Could not stat selected path, using parent directory`);
      dirToAdd = path.dirname(selectedPath); // Fallback to parent directory
    }

    addRecentDir(dirToAdd);

    let deeplinkData: WorkflowDeeplinkData | undefined = undefined;
    if (windowDeeplinkURL) {
      deeplinkData = parseWorkflowDeeplink(windowDeeplinkURL);
    }
    // Create a new window with the selected directory
    await createChat(
      app,
      undefined,
      dirToAdd,
      undefined,
      undefined,
      undefined,
      deeplinkData?.config,
      undefined,
      undefined,
      deeplinkData?.parameters
    );
  }
  return result;
};

// Global error handler
const handleFatalError = (error: Error) => {
  const windows = BrowserWindow.getAllWindows();
  windows.forEach((win) => {
    win.webContents.send('fatal-error', error.message || 'An unexpected error occurred');
  });
};

function sanitizeErrorForLogging(err: unknown): string {
  const msg = err instanceof Error ? err.message + '\n' + (err.stack || '') : String(err);
  return msg
    .replace(/sk-[a-zA-Z0-9]{20,}/g, 'sk-***')
    .replace(/[Aa]pi[_-]?[Kk]ey[=:]\s*\S+/g, 'api_key=***')
    .replace(/[Bb]earer\s+\S+/g, 'Bearer ***');
}

process.on('uncaughtException', (error) => {
  console.error('Uncaught Exception:', sanitizeErrorForLogging(error));
  handleFatalError(error);
});

process.on('unhandledRejection', (error) => {
  console.error('Unhandled Rejection:', sanitizeErrorForLogging(error));
  handleFatalError(error instanceof Error ? error : new Error(String(error)));
});

ipcMain.on('react-ready', (event) => {
  log.info('React ready event received');

  // Get the window that sent the react-ready event
  const window = BrowserWindow.fromWebContents(event.sender);
  const windowId = window?.id;

  // Send any pending initial message for this window
  if (windowId && pendingInitialMessages.has(windowId)) {
    const initialMessage = pendingInitialMessages.get(windowId)!;
    log.info('Sending pending initial message to window:', initialMessage);
    window.webContents.send('set-initial-message', initialMessage);
    pendingInitialMessages.delete(windowId);
  }

  if (pendingDeepLink && window) {
    log.info('Processing pending deep link:', pendingDeepLink);
    try {
      const parsedUrl = new URL(pendingDeepLink);
      if (parsedUrl.hostname === 'extension') {
        log.info('Sending add-extension IPC to ready window');
        window.webContents.send('add-extension', pendingDeepLink);
      } else if (parsedUrl.hostname === 'sessions') {
        log.info('Sending open-shared-session IPC to ready window');
        window.webContents.send('open-shared-session', pendingDeepLink);
      }
      pendingDeepLink = null;
    } catch (error) {
      log.error('Error processing pending deep link:', error);
      pendingDeepLink = null;
    }
  } else {
    log.info('No pending deep link to process');
  }

  if (pendingBrxtFilePath && window) {
    const filePath = pendingBrxtFilePath;
    pendingBrxtFilePath = null;
    log.info('Sending pending .brxt file to ready window:', filePath);
    window.webContents.send('open-brxt-file', filePath);
  }

  log.info('React ready - window is prepared for deep links');
});

ipcMain.handle('window:ensure-content-width', (event, minWidth: number) => {
  const win = BrowserWindow.fromWebContents(event.sender);
  if (!win || !Number.isFinite(minWidth)) {
    return { expanded: false, width: 0, height: 0 };
  }

  const currentContentBounds = win.getContentBounds();
  if (currentContentBounds.width >= minWidth || win.isMaximized() || win.isFullScreen()) {
    return {
      expanded: false,
      width: currentContentBounds.width,
      height: currentContentBounds.height,
    };
  }

  const windowBounds = win.getBounds();
  const display = screen.getDisplayMatching(windowBounds);
  const maxContentWidth = Math.max(720, display.workArea.width);
  const targetContentWidth = Math.min(Math.ceil(minWidth), maxContentWidth);

  win.setContentSize(targetContentWidth, currentContentBounds.height, true);

  const nextWindowBounds = win.getBounds();
  const maxX = display.workArea.x + display.workArea.width - nextWindowBounds.width;
  const maxY = display.workArea.y + display.workArea.height - nextWindowBounds.height;
  const adjustedX =
    maxX < display.workArea.x
      ? display.workArea.x
      : Math.min(Math.max(nextWindowBounds.x, display.workArea.x), maxX);
  const adjustedY =
    maxY < display.workArea.y
      ? display.workArea.y
      : Math.min(Math.max(nextWindowBounds.y, display.workArea.y), maxY);

  if (adjustedX !== nextWindowBounds.x || adjustedY !== nextWindowBounds.y) {
    win.setBounds({ ...nextWindowBounds, x: adjustedX, y: adjustedY }, true);
  }

  const nextContentBounds = win.getContentBounds();
  return {
    expanded: nextContentBounds.width > currentContentBounds.width,
    width: nextContentBounds.width,
    height: nextContentBounds.height,
  };
});

// ── The tab band's titlebar gestures ─────────────────────────────────────────
// Drag the empty part of the band to move the window; double-click it to zoom.
//
// ⚠ NOT `-webkit-app-region`, and that is the whole design. The empty area is
// empty because the tabs are not there YET, so a `drag` rect over it has a
// geometry that follows the tab list — and Blink only re-collects app-region
// rects on a paint lifecycle, so until that reaches this process macOS routes
// with the previous set and a just-created tab sits inside a stale drag rect.
// That regression is measured and pinned in
// `components/chatGroups/ChatTabStrip.appRegion.test.tsx`; the renderer half of
// this replacement is `useTabBandWindowGesture.ts`, the rules are in
// `titlebarWindowGesture.ts`.
const windowMoveDrag = new WindowMoveDragController({
  // DIP already, unlike a MouseEvent's `screenX` — so no coordinate crosses the
  // wire and `normalizeToDip` is not in the picture.
  cursorPoint: () => screen.getCursorScreenPoint(),
  schedule: (fn, ms) => setInterval(fn, ms),
  cancelScheduled: (timer) => {
    if (timer) clearInterval(timer as ReturnType<typeof setInterval>);
  },
});

function movableWindowFor(event: Electron.IpcMainEvent): MovableWindow | null {
  const win = BrowserWindow.fromWebContents(event.sender);
  // ONLY CHAT WINDOWS MOVE THIS WAY. Every window that loads `preload.js` can
  // reach this channel — launcher, artifact, app preview — and only a chat
  // window has a tab band to press, so a message from any of the others is a
  // window the gesture has no business moving. `windowMap` is exactly the set
  // `createChat` builds, which is the same membership test
  // `tab-drag:register-bands` uses for the same reason.
  if (!win || win.isDestroyed() || !windowMap.has(win.id)) return null;
  const sender = event.sender;
  return {
    id: win.id,
    // THE RENDERER COUNTS AS PART OF "ALIVE". It is the only thing that can
    // report the button coming up, so a window whose renderer has gone or
    // crashed must stop following the cursor even though the window itself is
    // perfectly healthy.
    isAlive: () => !win.isDestroyed() && !sender.isDestroyed() && !sender.isCrashed(),
    isFullScreen: () => win.isFullScreen(),
    isMaximized: () => win.isMaximized(),
    unmaximize: () => win.unmaximize(),
    getBounds: () => win.getBounds(),
    setPosition: (x, y) => win.setPosition(x, y),
  };
}

ipcMain.on('window:drag-start', (event) => {
  const win = movableWindowFor(event);
  if (!win) return;
  windowMoveDrag.begin(win);
});

ipcMain.on('window:drag-end', (event) => {
  const win = BrowserWindow.fromWebContents(event.sender);
  // Named, so a stale end from a window that is no longer the one being dragged
  // cannot cancel someone else's drag. The renderer sends this from four
  // redundant places on purpose.
  //
  // NO `windowMap` TEST HERE, unlike the two channels above and below, and that
  // is deliberate: this only ever STOPS a drag, `end` ignores an id that is not
  // the one being dragged, and a window outside `windowMap` could never have
  // started one. Gating it would be a way to STRAND a drag, never a way to
  // prevent one — including for a window that leaves the map mid-press.
  windowMoveDrag.end(win?.id);
});

ipcMain.on('window:toggle-zoom', (event) => {
  const win = BrowserWindow.fromWebContents(event.sender);
  // Chat windows only, for the reason spelled out on `movableWindowFor`: a
  // channel that resizes any window that loaded `preload.js` is broader than
  // the one gesture it exists for.
  if (!win || win.isDestroyed() || !windowMap.has(win.id)) return;
  // The press that opened this double-click also opened a drag. Ending it here
  // rather than trusting the renderer's `pointerup` to have landed first means
  // the timer can never be re-positioning the window against bounds that the
  // zoom below has already replaced.
  windowMoveDrag.end(win.id);
  // Leaving full screen belongs to the green button and to the menu, never to a
  // double-click on the titlebar — matching every other macOS window.
  if (win.isFullScreen()) return;

  let macPreference: string | null = null;
  if (process.platform === 'darwin') {
    try {
      // Desktop & Dock → "Double-click a window's title bar to". Unset by
      // default, and unset means Zoom.
      macPreference = systemPreferences.getUserDefault(
        'AppleActionOnDoubleClick',
        'string'
      ) as string;
    } catch {
      macPreference = null;
    }
  }

  const action = doubleClickWindowAction(process.platform, macPreference);
  if (action === 'none') return;
  if (action === 'minimize') {
    win.minimize();
    return;
  }
  if (win.isMaximized()) win.unmaximize();
  else win.maximize();
});

ipcMain.handle('open-external', async (event, url: string) => {
  try {
    const window = BrowserWindow.fromWebContents(event.sender);
    if (window) await openExternalBrowserNavigation(window, url);
  } catch (err) {
    console.error('open-external blocked:', err);
  }
});

ipcMain.handle('directory-chooser', async () => {
  return dialog.showOpenDialog({
    properties: ['openDirectory', 'createDirectory'],
    defaultPath: os.homedir(),
  });
});

ipcMain.handle('add-recent-dir', (_event, dir: string) => {
  if (!dir || typeof dir !== 'string') return;
  const normalized = path.resolve(dir);
  if (!path.isAbsolute(normalized)) return;
  addRecentDir(normalized);
});

// Handle scheduling engine settings
ipcMain.handle('get-settings', () => {
  try {
    return loadSettings();
  } catch (error) {
    console.error('Error getting settings:', error);
    return null;
  }
});

ipcMain.handle('save-settings', (_event, settings) => {
  if (typeof settings !== 'object' || settings === null || Array.isArray(settings)) {
    console.error('save-settings: invalid settings object received');
    return;
  }
  try {
    saveSettings(settings);
    return true;
  } catch (error) {
    console.error('Error saving settings:', error);
    return false;
  }
});

ipcMain.handle('get-secret-key', () => {
  const settings = loadSettings();
  return getServerSecret(settings);
});

// Issue #56 DR-16. The renderer IS the user's surface, so it is the one holder
// of the raw key besides this process. It attaches it to the three tier-raising
// requests only — never as a default header on every request, which would make
// the proof as ambient as the daemon secret it is meant to be stronger than.
ipcMain.handle('get-user-action-key', () => {
  const settings = loadSettings();
  return getUserActionKey(settings);
});

ipcMain.handle('get-biorouterd-host-port', async (event) => {
  const windowId = BrowserWindow.fromWebContents(event.sender)?.id;
  if (!windowId) {
    return null;
  }
  const client = biorouterdClients.get(windowId);
  if (!client) {
    return null;
  }
  return client.getConfig().baseUrl || null;
});

// Handle menu bar icon visibility
ipcMain.handle('set-menu-bar-icon', async (_event, show: boolean) => {
  try {
    const settings = loadSettings();
    settings.showMenuBarIcon = show;
    saveSettings(settings);

    if (show) {
      createTray();
    } else {
      destroyTray();
    }
    return true;
  } catch (error) {
    console.error('Error setting menu bar icon:', error);
    return false;
  }
});

ipcMain.handle('get-menu-bar-icon-state', () => {
  try {
    const settings = loadSettings();
    return settings.showMenuBarIcon ?? true;
  } catch (error) {
    console.error('Error getting menu bar icon state:', error);
    return true;
  }
});

// Handle dock icon visibility (macOS only)
ipcMain.handle('set-dock-icon', async (_event, show: boolean) => {
  try {
    if (process.platform !== 'darwin') return false;

    const settings = loadSettings();
    settings.showDockIcon = show;
    saveSettings(settings);

    if (show) {
      app.dock?.show();
    } else {
      // Only hide the dock if we have a menu bar icon to maintain accessibility
      if (settings.showMenuBarIcon) {
        app.dock?.hide();
        setTimeout(() => {
          focusWindow();
        }, 50);
      }
    }
    return true;
  } catch (error) {
    console.error('Error setting dock icon:', error);
    return false;
  }
});

ipcMain.handle('get-dock-icon-state', () => {
  try {
    if (process.platform !== 'darwin') return true;
    const settings = loadSettings();
    return settings.showDockIcon ?? true;
  } catch (error) {
    console.error('Error getting dock icon state:', error);
    return true;
  }
});

// Handle opening system notifications preferences
ipcMain.handle('open-notifications-settings', async () => {
  try {
    if (process.platform === 'darwin') {
      spawn('open', ['x-apple.systempreferences:com.apple.preference.notifications']);
      return true;
    } else if (process.platform === 'win32') {
      // Windows: Open notification settings in Settings app
      await shell.openExternal('ms-settings:notifications');
      return true;
    } else if (process.platform === 'linux') {
      // Linux: Try different desktop environments
      // GNOME
      try {
        spawn('gnome-control-center', ['notifications']);
        return true;
      } catch {
        console.log('GNOME control center not found, trying other options');
      }

      // KDE Plasma
      try {
        spawn('systemsettings5', ['kcm_notifications']);
        return true;
      } catch {
        console.log('KDE systemsettings5 not found, trying other options');
      }

      // XFCE
      try {
        spawn('xfce4-settings-manager', ['--socket-id=notifications']);
        return true;
      } catch {
        console.log('XFCE settings manager not found, trying other options');
      }

      // Fallback: Try to open general settings
      try {
        spawn('gnome-control-center');
        return true;
      } catch {
        console.warn('Could not find a suitable settings application for Linux');
        return false;
      }
    } else {
      console.warn(
        `Opening notification settings is not supported on platform: ${process.platform}`
      );
      return false;
    }
  } catch (error) {
    console.error('Error opening notification settings:', error);
    return false;
  }
});

// Handle wakelock setting
ipcMain.handle('set-wakelock', async (_event, enable: boolean) => {
  try {
    const settings = loadSettings();
    settings.enableWakelock = enable;
    saveSettings(settings);

    // Stop all existing power save blockers when disabling the setting
    if (!enable) {
      for (const [windowId, blockerId] of windowPowerSaveBlockers.entries()) {
        try {
          powerSaveBlocker.stop(blockerId);
          console.log(
            `[Main] Stopped power save blocker ${blockerId} for window ${windowId} due to wakelock setting disabled`
          );
        } catch (error) {
          console.error(
            `[Main] Failed to stop power save blocker ${blockerId} for window ${windowId}:`,
            error
          );
        }
      }
      windowPowerSaveBlockers.clear();
    }

    return true;
  } catch (error) {
    console.error('Error setting wakelock:', error);
    return false;
  }
});

ipcMain.handle('get-wakelock-state', () => {
  try {
    const settings = loadSettings();
    return settings.enableWakelock ?? false;
  } catch (error) {
    console.error('Error getting wakelock state:', error);
    return false;
  }
});

ipcMain.handle('set-spellcheck', async (_event, enable: boolean) => {
  try {
    const settings = loadSettings();
    settings.spellcheckEnabled = enable;
    saveSettings(settings);
    return true;
  } catch (error) {
    console.error('Error setting spellcheck:', error);
    return false;
  }
});

ipcMain.handle('get-spellcheck-state', () => {
  try {
    const settings = loadSettings();
    return settings.spellcheckEnabled ?? true;
  } catch (error) {
    console.error('Error getting spellcheck state:', error);
    return true;
  }
});

// Add file/directory selection handler
ipcMain.handle('select-file-or-directory', async (_event, defaultPath?: string) => {
  if (process.env.PLAYWRIGHT_SELECT_PATH) {
    return process.env.PLAYWRIGHT_SELECT_PATH;
  }

  const dialogOptions: OpenDialogOptions = {
    properties: process.platform === 'darwin' ? ['openFile', 'openDirectory'] : ['openFile'],
  };

  // Set default path if provided
  if (defaultPath) {
    // Expand tilde to home directory
    const expandedPath = expandTilde(defaultPath);

    // Check if the path exists
    try {
      const stats = await fs.stat(expandedPath);
      if (stats.isDirectory()) {
        dialogOptions.defaultPath = expandedPath;
      } else {
        dialogOptions.defaultPath = path.dirname(expandedPath);
      }
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
    } catch (error) {
      // If path doesn't exist, fall back to home directory and log error
      console.error(`Default path does not exist: ${expandedPath}, falling back to home directory`);
      dialogOptions.defaultPath = os.homedir();
    }
  }

  const result = (await dialog.showOpenDialog(dialogOptions)) as unknown as OpenDialogReturnValue;

  if (!result.canceled && result.filePaths.length > 0) {
    return result.filePaths[0];
  }
  return null;
});

// Import session: open native file dialog, read JSON, return content
ipcMain.handle('import-session-file', async () => {
  const result = (await dialog.showOpenDialog({
    properties: ['openFile'],
    filters: [{ name: 'JSON', extensions: ['json'] }],
  })) as unknown as OpenDialogReturnValue;

  if (result.canceled || result.filePaths.length === 0) return null;
  return fs.readFile(result.filePaths[0], 'utf-8');
});

// IPC handler to save data URL to a temporary file
ipcMain.handle('save-data-url-to-temp', async (_event, dataUrl: string, uniqueId: string) => {
  console.log(`[Main] Received save-data-url-to-temp for ID: ${uniqueId}`);
  try {
    // Input validation for uniqueId - only allow alphanumeric characters and hyphens
    if (!uniqueId || !/^[a-zA-Z0-9-]+$/.test(uniqueId) || uniqueId.length > 50) {
      console.error('[Main] Invalid uniqueId format received.');
      return { id: uniqueId, error: 'Invalid uniqueId format' };
    }

    // Input validation for dataUrl. The 10 MB cap (matching the renderer-side
    // image limit ~5 MB after base64 overhead) caused main-process heap spikes
    // when users rapidly pasted screenshots — every IPC call materializes the
    // entire string in main via structured clone before validation. Drop to
    // 4 MB (≈3 MB image payload), which still covers typical screenshots.
    if (!dataUrl || typeof dataUrl !== 'string' || dataUrl.length > 4 * 1024 * 1024) {
      console.error('[Main] Invalid or too large data URL received.');
      return { id: uniqueId, error: 'Invalid or too large data URL' };
    }

    const tempDir = await ensureTempDirExists();
    const matches = dataUrl.match(/^data:(image\/(png|jpeg|jpg|gif|webp));base64,(.*)$/);

    if (!matches || matches.length < 4) {
      console.error('[Main] Invalid data URL format received.');
      return { id: uniqueId, error: 'Invalid data URL format or unsupported image type' };
    }

    const imageExtension = matches[2]; // e.g., "png", "jpeg"
    const base64Data = matches[3];

    // Validate base64 data
    if (!base64Data || !/^[A-Za-z0-9+/]*={0,2}$/.test(base64Data)) {
      console.error('[Main] Invalid base64 data received.');
      return { id: uniqueId, error: 'Invalid base64 data' };
    }

    const buffer = Buffer.from(base64Data, 'base64');

    // Validate image size (max 5MB)
    if (buffer.length > 3 * 1024 * 1024) {
      console.error('[Main] Image too large.');
      return { id: uniqueId, error: 'Image too large (max 3MB)' };
    }

    const randomString = crypto.randomBytes(8).toString('hex');
    const fileName = `pasted-${uniqueId}-${randomString}.${imageExtension}`;
    const filePath = path.join(tempDir, fileName);

    // Ensure the resolved path is still within the temp directory
    const resolvedPath = path.resolve(filePath);
    const resolvedTempDir = path.resolve(tempDir);
    if (!resolvedPath.startsWith(resolvedTempDir + path.sep)) {
      console.error('[Main] Attempted path traversal detected.');
      return { id: uniqueId, error: 'Invalid file path' };
    }

    await fs.writeFile(filePath, buffer, { mode: 0o600 });
    console.log(`[Main] Saved image for ID ${uniqueId} to: ${filePath}`);
    return { id: uniqueId, filePath: filePath };
  } catch (error) {
    console.error(`[Main] Failed to save image to temp for ID ${uniqueId}:`, error);
    return { id: uniqueId, error: error instanceof Error ? error.message : 'Failed to save image' };
  }
});

// IPC handler to serve temporary image files
ipcMain.handle('get-temp-image', async (_event, filePath: string) => {
  try {
    const { buffer, mimeType } = await readTrustedTempImage(filePath, 16 * 1024 * 1024);
    return `data:${mimeType};base64,${buffer.toString('base64')}`;
  } catch {
    return null;
  }
});
ipcMain.on('delete-temp-file', async (_event, filePath: string) => {
  console.log(`[Main] Received delete-temp-file for path: ${filePath}`);

  // Input validation
  if (!filePath || typeof filePath !== 'string') {
    console.warn('[Main] Invalid file path provided for deletion');
    return;
  }

  // Ensure the path is within the designated temp directory
  const resolvedPath = path.resolve(filePath);
  const resolvedTempDir = path.resolve(biorouterTempDir);

  if (!resolvedPath.startsWith(resolvedTempDir + path.sep)) {
    console.warn(`[Main] Attempted to delete file outside designated temp directory: ${filePath}`);
    return;
  }

  try {
    // Check if it's a regular file first, before trying realpath
    const stats = await fs.lstat(filePath);
    if (!stats.isFile()) {
      console.warn(`[Main] Not a regular file, refusing to delete: ${filePath}`);
      return;
    }

    // Get the real paths for both the temp directory and the file to handle symlinks properly
    let actualPath = filePath;

    try {
      const realTempDir = await fs.realpath(biorouterTempDir);
      const realPath = await fs.realpath(filePath);

      // Double-check that the real path is still within our real temp directory
      if (!realPath.startsWith(realTempDir + path.sep)) {
        console.warn(
          `[Main] Real path is outside designated temp directory: ${realPath} not in ${realTempDir}`
        );
        return;
      }
      actualPath = realPath;
    } catch (realpathError) {
      // If realpath fails, use the original path validation
      console.log(
        `[Main] realpath failed for ${filePath}, using original path validation:`,
        realpathError instanceof Error ? realpathError.message : String(realpathError)
      );
    }

    await fs.unlink(actualPath);
    console.log(`[Main] Deleted temp file: ${filePath}`);
  } catch (error) {
    if (error && typeof error === 'object' && 'code' in error && error.code !== 'ENOENT') {
      // ENOENT means file doesn't exist, which is fine
      console.error(`[Main] Failed to delete temp file: ${filePath}`, error);
    } else {
      console.log(`[Main] Temp file already deleted or not found: ${filePath}`);
    }
  }
});

function tempImageMimeType(filePath: string): string | null {
  const extension = path.extname(filePath).slice(1).toLowerCase();
  if (!['png', 'jpg', 'jpeg', 'gif', 'webp'].includes(extension)) return null;
  return IMAGE_MIME_TYPES[extension] ?? null;
}

function hasImageSignature(buffer: Buffer, mimeType: string): boolean {
  if (mimeType === 'image/png')
    return buffer.subarray(0, 8).equals(Buffer.from('89504e470d0a1a0a', 'hex'));
  if (mimeType === 'image/jpeg') return buffer[0] === 0xff && buffer[1] === 0xd8;
  if (mimeType === 'image/gif') return buffer.subarray(0, 4).toString('ascii') === 'GIF8';
  return (
    mimeType === 'image/webp' &&
    buffer.subarray(0, 4).toString('ascii') === 'RIFF' &&
    buffer.subarray(8, 12).toString('ascii') === 'WEBP'
  );
}

async function readTrustedTempImage(filePath: string, maxBytes: number) {
  if (!filePath || typeof filePath !== 'string') {
    throw new Error('Invalid file path provided');
  }
  const resolvedPath = path.resolve(filePath);
  const resolvedTempDir = path.resolve(biorouterTempDir);
  if (!resolvedPath.startsWith(resolvedTempDir + path.sep)) {
    throw new Error('File path is outside the designated temp directory');
  }
  const linkStats = await fs.lstat(resolvedPath);
  if (!linkStats.isFile() || linkStats.isSymbolicLink()) {
    throw new Error('Path is not a regular file');
  }
  const realTempDir = await fs.realpath(biorouterTempDir);
  const realPath = await fs.realpath(resolvedPath);
  if (!realPath.startsWith(realTempDir + path.sep)) {
    throw new Error('File path resolves outside the designated temp directory');
  }
  const mimeType = tempImageMimeType(realPath);
  if (!mimeType) {
    throw new Error('Unsupported image type');
  }
  const handle = await fs.open(
    realPath,
    fsSync.constants.O_RDONLY | (fsSync.constants.O_NOFOLLOW ?? 0)
  );
  try {
    const stats = await handle.stat().catch(async (error) => {
      await handle.close();
      throw error;
    });
    if (!stats.isFile() || stats.size <= 0 || stats.size > maxBytes) {
      throw new Error('Temporary image exceeds the allowed size');
    }
    const buffer = await handle.readFile();
    if (!hasImageSignature(buffer, mimeType)) throw new Error('Invalid image content');
    return { buffer, mimeType };
  } finally {
    await handle.close();
  }
}

// IPC handler to read a temporary image file and return raw base64 + mimeType
ipcMain.handle('read-temp-image-as-base64', async (_event, filePath: string) => {
  const { buffer, mimeType } = await readTrustedTempImage(filePath, 8 * 1024 * 1024);
  return { data: buffer.toString('base64'), mimeType };
});

ipcMain.handle('check-ollama', async () => {
  try {
    return new Promise((resolve) => {
      // Run `ps` and filter for "ollama"
      const ps = spawn('ps', ['aux']);
      const grep = spawn('grep', ['-iw', '[o]llama']);

      let output = '';
      let errorOutput = '';

      // Pipe ps output to grep
      ps.stdout.pipe(grep.stdin);

      grep.stdout.on('data', (data) => {
        output += data.toString();
      });

      grep.stderr.on('data', (data) => {
        errorOutput += data.toString();
      });

      grep.on('close', (code) => {
        if (code !== null && code !== 0 && code !== 1) {
          // grep returns 1 when no matches found
          console.error('Error executing grep command:', errorOutput);
          return resolve(false);
        }

        console.log('Raw stdout from ps|grep command:', output);
        const trimmedOutput = output.trim();
        console.log('Trimmed stdout:', trimmedOutput);

        const isRunning = trimmedOutput.length > 0;
        resolve(isRunning);
      });

      ps.on('error', (error) => {
        console.error('Error executing ps command:', error);
        resolve(false);
      });

      grep.on('error', (error) => {
        console.error('Error executing grep command:', error);
        resolve(false);
      });

      // Close ps stdin when done
      ps.stdout.on('end', () => {
        grep.stdin.end();
      });
    });
  } catch (err) {
    console.error('Error checking for Ollama:', err);
    return false;
  }
});

ipcMain.handle('read-file', async (event, filePath) => {
  const expandedPath = expandBiorouterPath(filePath);
  try {
    const resolvedPath = path.resolve(expandedPath);
    if (!isAllowedFilePath(resolvedPath, workingDirForSender(event))) {
      throw new Error(`Access denied: path '${resolvedPath}' is outside allowed directories`);
    }
    // Single fs.readFile path for all platforms. The previous `spawn('cat')`
    // fallback added an extra process per call (FD + PID pressure) for no
    // benefit — fs.readFile is faster and doesn't depend on `cat` being on
    // PATH inside the Electron environment.
    const buffer = await fs.readFile(expandedPath);
    return { file: buffer.toString('utf8'), filePath: expandedPath, error: null, found: true };
  } catch (error) {
    const fileError = error as { code?: string };
    if (fileError.code !== 'ENOENT') {
      console.error('Error reading file:', error);
    }
    return { file: '', filePath: expandedPath, error, found: false };
  }
});

/**
 * A PNG of a rectangle of this window, written to the temp dir.
 *
 * `capturePage` is a **compositor** grab, not a DOM walk, and that is the whole
 * reason this exists: the artifact preview is a `srcdoc` iframe sandboxed
 * without `allow-same-origin`, and the embedded browser is a separate native
 * view — neither is reachable by html2canvas or any of its relatives, which
 * re-render the DOM from the host's side. Verified against this exact Electron:
 * a lime rect painted inside a sandboxed frame comes back lime.
 *
 * The bytes go to a file rather than across IPC because the same image has to
 * reach the agent, and the workspace channel caps an inbound frame at 128 KiB.
 */
ipcMain.handle(
  'capture-region',
  async (
    event,
    payload: {
      x: number;
      y: number;
      width: number;
      height: number;
      label?: string;
      containment?: { x: number; y: number; width: number; height: number };
    }
  ) => {
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window) return null;
    const numbers = [payload?.x, payload?.y, payload?.width, payload?.height];
    if (!numbers.every((value) => Number.isFinite(value))) return null;
    const width = Math.round(payload.width);
    const height = Math.round(payload.height);
    if (width <= 0 || height <= 0 || width > 8192 || height > 8192 || width * height > 32_000_000) {
      return null;
    }
    const x = Math.round(payload.x);
    const y = Math.round(payload.y);
    const contentBounds = window.getContentBounds();
    if (x < 0 || y < 0 || x + width > contentBounds.width || y + height > contentBounds.height) {
      return null;
    }
    if (payload.containment) {
      const containmentNumbers = [
        payload.containment.x,
        payload.containment.y,
        payload.containment.width,
        payload.containment.height,
      ];
      if (!containmentNumbers.every((value) => Number.isFinite(value))) return null;
      const left = Math.round(payload.containment.x);
      const top = Math.round(payload.containment.y);
      const right = Math.round(payload.containment.x + payload.containment.width);
      const bottom = Math.round(payload.containment.y + payload.containment.height);
      if (
        payload.containment.width <= 0 ||
        payload.containment.height <= 0 ||
        x < left ||
        y < top ||
        x + width > right ||
        y + height > bottom
      ) {
        return null;
      }
    }

    const image = await window.webContents.capturePage({
      x,
      y,
      width,
      height,
    });
    // A hidden-then-navigated view yields a zero-byte image and does NOT
    // reject, so the empty case has to be checked rather than assumed.
    if (image.isEmpty()) return null;

    const dir = await ensureTempDirExists();
    const safeLabel = (payload.label ?? 'region').replace(/[^a-zA-Z0-9-]/g, '').slice(0, 24);
    const filePath = path.join(
      dir,
      `capture-${safeLabel || 'region'}-${crypto.randomBytes(6).toString('hex')}.png`
    );
    await fs.writeFile(filePath, image.toPNG(), { mode: 0o600 });
    const size = image.getSize();
    return { path: filePath, width: size.width, height: size.height };
  }
);

// ── Embedded browser (the artifact panel's live web view) ────────────────────
//
// Every handler resolves the owning window from the *event sender*. The main
// registry keys renderer view ids by that owner, so an identical React id in a
// second window cannot drive, read, capture, or destroy the first window's view.
ipcMain.handle('embedded-browser:is-managed-app-url', (event, payload: { url: string }) => {
  const window = BrowserWindow.fromWebContents(event.sender);
  if (!window || typeof payload?.url !== 'string') return false;
  return Boolean(managedAppPreviewScope(payload.url, managedAppPreviewBackends.get(window.id)));
});

ipcMain.handle(
  'embedded-browser:create',
  (event, payload: { viewId: string; url: string; managedOnly?: boolean }) => {
    const window = BrowserWindow.fromWebContents(event.sender);
    if (
      !window ||
      typeof payload?.viewId !== 'string' ||
      (payload.managedOnly !== undefined && typeof payload.managedOnly !== 'boolean')
    )
      return null;
    return createEmbeddedBrowser(
      window,
      payload.viewId,
      payload.url,
      (state) => {
        if (!event.sender.isDestroyed()) {
          event.sender.send('embedded-browser:state', { viewId: payload.viewId, state });
        }
      },
      managedAppPreviewBackends.get(window.id),
      payload.managedOnly === true
    );
  }
);

ipcMain.handle(
  'embedded-browser:set-bounds',
  (event, payload: { viewId: string; bounds: EmbeddedBrowserBounds }) => {
    const window = BrowserWindow.fromWebContents(event.sender);
    if (window) setEmbeddedBrowserBounds(window, payload.viewId, payload.bounds);
  }
);

ipcMain.handle(
  'embedded-browser:set-visible',
  (event, payload: { viewId: string; visible: boolean }) => {
    const window = BrowserWindow.fromWebContents(event.sender);
    if (window) setEmbeddedBrowserVisible(window, payload.viewId, payload.visible);
  }
);

ipcMain.handle('embedded-browser:navigate', (event, payload: { viewId: string; url: string }) => {
  const window = BrowserWindow.fromWebContents(event.sender);
  return window ? navigateEmbeddedBrowser(window, payload.viewId, payload.url) : false;
});

ipcMain.handle(
  'embedded-browser:control',
  (
    event,
    payload: { viewId: string; action: 'back' | 'forward' | 'reload' | 'stop' | 'reload-if-idle' }
  ) => {
    const window = BrowserWindow.fromWebContents(event.sender);
    return window ? controlEmbeddedBrowser(window, payload.viewId, payload.action) : false;
  }
);

/**
 * Text and pixels from the embedded browser.
 *
 * ⚠ These exist *separately* from `capture-region` because a `WebContentsView`
 * is its own `WebContents`, a sibling native layer rather than part of the host
 * page. `capturePage` on the window composites the window's own document —
 * including sandboxed iframes, which is verified — but **not** a child view. A
 * live page therefore has to be captured through its own contents.
 */
ipcMain.handle(
  'embedded-browser:read-text',
  async (event, payload: { viewId: string; maxChars?: number }) => {
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window) return null;
    const requested = payload.maxChars ?? 20_000;
    const limit = Number.isFinite(requested)
      ? Math.min(40_000, Math.max(0, Math.floor(requested)))
      : 20_000;
    return readEmbeddedBrowserText(window, payload.viewId, limit);
  }
);

ipcMain.handle('embedded-browser:capture', async (event, payload: { viewId: string }) => {
  const window = BrowserWindow.fromWebContents(event.sender);
  if (!window) return null;
  const shot = await captureEmbeddedBrowser(window, payload.viewId);
  if (!shot) return null;
  const dir = await ensureTempDirExists();
  const filePath = path.join(dir, `capture-page-${crypto.randomBytes(6).toString('hex')}.png`);
  await fs.writeFile(filePath, shot.png, { mode: 0o600 });
  return {
    path: filePath,
    width: shot.width,
    height: shot.height,
    sourceRevision: shot.sourceRevision,
  };
});

ipcMain.handle('embedded-browser:clear-data', async (event, payload: { viewId: string }) => {
  const window = BrowserWindow.fromWebContents(event.sender);
  return window ? clearEmbeddedBrowserData(window, payload.viewId) : false;
});

ipcMain.handle('embedded-browser:destroy', (event, payload: { viewId: string }) => {
  const window = BrowserWindow.fromWebContents(event.sender);
  if (window) destroyEmbeddedBrowser(window, payload.viewId);
});

ipcMain.handle('read-artifact-file', async (event, filePath: string) => {
  const expandedPath = expandBiorouterPath(filePath);
  const resolvedPath = path.resolve(expandedPath);
  const title = path.basename(resolvedPath) || resolvedPath;
  try {
    if (!isAllowedFilePath(resolvedPath, workingDirForSender(event))) {
      throw new Error(`Access denied: path '${resolvedPath}' is outside allowed directories`);
    }

    const pathStats = await fs.lstat(resolvedPath);
    if (pathStats.isSymbolicLink()) {
      throw new Error('Symbolic links cannot be previewed');
    }
    if (pathStats.isDirectory()) {
      const gitTree = await readGitArtifactTree(resolvedPath);
      if (gitTree) {
        return {
          kind: 'gitDirectory',
          title,
          path: resolvedPath,
          branch: gitTree.branch,
          entries: gitTree.entries,
          found: true,
        };
      }
      return {
        kind: 'directory',
        title,
        path: resolvedPath,
        found: true,
        entries: await readArtifactDirectoryTree(resolvedPath),
      };
    }

    if (!pathStats.isFile()) {
      throw new Error('Path is not a regular file or directory');
    }

    const handle = await fs.open(
      resolvedPath,
      fsSync.constants.O_RDONLY | (fsSync.constants.O_NOFOLLOW ?? 0)
    );
    const stats = await handle.stat();

    const mimeType = mimeTypeForArtifactPath(resolvedPath);

    // Artifacts are auto-detected from assistant text and opened without a
    // click, so a model that names a huge file must not be able to make the
    // main process buffer it (images additionally grow ~4/3 as base64).
    // Report oversized files as binary: the UI shows metadata, not content.
    if (stats.size > ARTIFACT_PREVIEW_MAX_BYTES) {
      await handle.close();
      return {
        kind: 'binary',
        title,
        path: resolvedPath,
        mimeType,
        size: stats.size,
        found: true,
      };
    }

    let buffer: Buffer;
    try {
      buffer = await readFileHandleBounded(handle, ARTIFACT_PREVIEW_MAX_BYTES);
    } finally {
      await handle.close();
    }
    const revision = artifactSourceRevision(stats.size, stats.mtimeMs, buffer);
    const documentFormat = documentFormatForArtifactPath(resolvedPath);
    if (documentFormat) {
      let officeText: { text: string; truncated: boolean } | null = null;
      if (documentFormat !== 'pdf') {
        const zip = validatedOfficeZip(buffer);
        validateOfficeDocumentShape(zip, documentFormat);
        officeText = extractOfficeText(zip, documentFormat);
      }
      return {
        kind: 'document',
        format: documentFormat,
        title,
        path: resolvedPath,
        mimeType,
        data: Uint8Array.from(buffer).buffer,
        size: stats.size,
        revision,
        ...(officeText
          ? { extractedText: officeText.text, textTruncated: officeText.truncated }
          : {}),
        found: true,
      };
    }

    if (mimeType.startsWith('image/')) {
      // HEIC has no decoder in any browser, so it is converted here through the
      // OS's own — the one route that carries neither a copyleft obligation nor
      // HEVC patent exposure. Where that is unavailable the preview falls
      // through to a card that names the format, which is more useful than a
      // broken image.
      if (mimeType === 'image/heic' || mimeType === 'image/heif') {
        const png = await heicToPng(resolvedPath);
        if (png) {
          assertSafeRasterImageDimensions(png, 'image/png');
          return {
            kind: 'image',
            title,
            path: resolvedPath,
            mimeType: 'image/png',
            bytes: Uint8Array.from(png).buffer,
            size: stats.size,
            revision,
            found: true,
          };
        }
        return {
          kind: 'binary',
          title,
          path: resolvedPath,
          mimeType,
          size: stats.size,
          found: true,
        };
      }

      // Small images travel as a data URL because a `blob:` needs revoking and
      // that bookkeeping is not worth it for an icon. Large ones travel as raw
      // bytes: base64 would cost ~4/3 of the file as a JS string, twice.
      // TIFF always takes the bytes path — the renderer has to decode it before
      // anything can be shown, so a data URL would be pure waste.
      assertSafeRasterImageDimensions(buffer, mimeType);
      const asBytes = stats.size > IMAGE_BLOB_URL_THRESHOLD_BYTES || mimeType === 'image/tiff';
      return {
        kind: 'image',
        title,
        path: resolvedPath,
        mimeType,
        ...(asBytes
          ? { bytes: Uint8Array.from(buffer).buffer }
          : { dataUrl: `data:${mimeType};base64,${buffer.toString('base64')}` }),
        size: stats.size,
        revision,
        found: true,
      };
    }

    if (isTextArtifact(mimeType, buffer)) {
      return {
        kind: mimeType === 'text/html' ? 'html' : 'text',
        title,
        path: resolvedPath,
        mimeType,
        text: buffer.toString('utf8'),
        size: stats.size,
        revision,
        found: true,
      };
    }

    return {
      kind: 'binary',
      title,
      path: resolvedPath,
      mimeType,
      size: stats.size,
      found: true,
    };
  } catch (error) {
    const fallback = error instanceof Error ? error.message : 'Could not read artifact file.';
    // A moved/deleted file is an expected state here (the sibling read-file
    // handler already treats ENOENT as non-exceptional): return a friendly
    // message plus the structured code, never the raw Node errno string (#36).
    const fileError = error as { code?: string } | null;
    const friendly = friendlyArtifactFileError(fileError?.code, fallback);
    return {
      kind: 'error',
      title,
      path: resolvedPath,
      error: friendly.message,
      code: friendly.code,
      found: false,
    };
  }
});

// --- Does a linked file actually exist? --------------------------------------
//
// The assistant names paths it never created, and the renderer used to paint
// every one of them as a clickable accent-coloured link. These few helpers
// answer the one question that separates a real file from a described one, and
// they are written so the answer holds on any host OS: no `/` is assumed
// anywhere, every join goes through `node:path`, and the Windows drive-letter
// and UNC forms `parseFileLink` accepts in the renderer are recognised here too.

/** `C:\…`, `C:/…` and `\\server\share\…`. Recognised on EVERY platform, so a
 *  Windows path is never quietly grafted onto a POSIX working directory. */
const WINDOWS_ABSOLUTE_PATH = /^(?:[A-Za-z]:[\\/]|\\\\)/;

function isAbsoluteOnAnyPlatform(candidate: string): boolean {
  return (
    path.isAbsolute(candidate) || candidate.startsWith('/') || WINDOWS_ABSOLUTE_PATH.test(candidate)
  );
}

/**
 * The user's home directory, environment first: `$HOME` on posix,
 * `%USERPROFILE%` on win32, and the OS's own record when neither is set.
 */
function homeDirectory(): string {
  const fromEnvironment = process.platform === 'win32' ? process.env.USERPROFILE : process.env.HOME;
  if (fromEnvironment && fromEnvironment.trim()) return fromEnvironment;
  try {
    return os.homedir();
  } catch {
    return '';
  }
}

/** Expand a leading `~`, `~/` or `~\` against {@link homeDirectory}. */
function expandHomePrefix(candidate: string): string {
  if (candidate === '~') return homeDirectory() || candidate;
  if (!candidate.startsWith('~/') && !candidate.startsWith('~\\')) return candidate;
  const home = homeDirectory();
  return home ? path.join(home, candidate.slice(2)) : candidate;
}

/**
 * The absolute path the preview panel would read for this link, or null when it
 * cannot be located at all.
 *
 * The composition matters more than any single step: `expandHomePrefix` +
 * `reinterpretTildeAsAbsolute` + `expandBiorouterPath` is exactly what
 * `expandBiorouterPath` alone does to the same string in `read-artifact-file`,
 * with the home reading taken from the environment rather than straight from
 * `os.homedir()`. Agreement with that handler is the whole point — an "exists"
 * verdict reached down a *different* resolution than the click will take is
 * worse than no verdict, because it paints an orange link onto a file the panel
 * then fails to open.
 */
function resolveCheckedFilePath(rawPath: unknown, rawWorkingDir: unknown): string | null {
  if (typeof rawPath !== 'string') return null;
  const candidate = rawPath.trim();
  if (!candidate || candidate.includes('\0')) return null;

  const expanded = expandBiorouterPath(
    reinterpretTildeAsAbsolute(candidate, expandHomePrefix(candidate), (probe) =>
      fsSync.existsSync(probe)
    )
  );

  if (WINDOWS_ABSOLUTE_PATH.test(expanded)) {
    // Only a Windows host can say anything about a drive-letter or UNC path.
    // Elsewhere `path.resolve` would silently graft it onto the process cwd and
    // stat something unrelated, so answer "cannot locate" instead.
    return process.platform === 'win32' ? path.resolve(expanded) : null;
  }
  if (isAbsoluteOnAnyPlatform(expanded)) return path.resolve(expanded);

  const workingDir = typeof rawWorkingDir === 'string' ? rawWorkingDir.trim() : '';
  if (!workingDir) return null;
  const base = expandBiorouterPath(expandHomePrefix(workingDir));
  if (!isAbsoluteOnAnyPlatform(base)) return null;
  if (WINDOWS_ABSOLUTE_PATH.test(base) && process.platform !== 'win32') return null;
  return path.resolve(base, expanded);
}

/** A message can name a lot of paths; it cannot name an unbounded number. */
const MAX_FILE_PATH_CHECKS = 512;
const FILE_PATH_CHECK_MISS = { exists: false, isDirectory: false };

/**
 * Existence of a batch of paths, one answer per request, in order.
 *
 * The reply is two booleans and nothing else — no contents, no directory
 * listing, no error text — so it cannot serve as a weaker read channel beside
 * `read-artifact-file`. "Exists" deliberately means *"the preview panel could
 * show this"*, which is the question a link is really asking: a path the
 * allowlist denies and a path that was deleted both answer no, and a symlink
 * answers no because `read-artifact-file` refuses to preview one. That also
 * stops this becoming an existence oracle for `~/.ssh` and friends.
 */
ipcMain.handle('check-file-paths', async (event, requests: unknown) => {
  if (!Array.isArray(requests)) return [];
  const sessionWorkingDir = workingDirForSender(event);
  return Promise.all(
    requests.slice(0, MAX_FILE_PATH_CHECKS).map(async (request) => {
      const entry = (request ?? {}) as { path?: unknown; workingDir?: unknown };
      const resolvedPath = resolveCheckedFilePath(entry.path, entry.workingDir);
      if (!resolvedPath) return FILE_PATH_CHECK_MISS;
      if (!isAllowedFilePath(resolvedPath, sessionWorkingDir)) return FILE_PATH_CHECK_MISS;
      try {
        const stats = await fs.lstat(resolvedPath);
        if (stats.isSymbolicLink()) return FILE_PATH_CHECK_MISS;
        return { exists: stats.isFile() || stats.isDirectory(), isDirectory: stats.isDirectory() };
      } catch {
        return FILE_PATH_CHECK_MISS;
      }
    })
  );
});

ipcMain.handle('write-file', async (_event, filePath, content) => {
  try {
    const expandedPath = expandBiorouterPath(filePath);
    await fs.mkdir(path.dirname(expandedPath), { recursive: true });
    // Atomic replace via a uniquely named temp file + rename, so a concurrent
    // reader (e.g. the CLI reading skills-config.json) never observes a
    // truncated half-written file, and two writers never share a temp path.
    const tmpPath = `${expandedPath}.${process.pid}.${Date.now()}.${Math.random().toString(36).slice(2)}.tmp`;
    await fs.writeFile(tmpPath, content, { encoding: 'utf8' });
    try {
      await fs.rename(tmpPath, expandedPath);
    } catch (renameError) {
      await fs.unlink(tmpPath).catch(() => {});
      throw renameError;
    }
    return true;
  } catch (error) {
    console.error('Error writing to file:', error);
    return false;
  }
});

// Enhanced file operations
ipcMain.handle('ensure-directory', async (_event, dirPath) => {
  try {
    const expandedPath = expandBiorouterPath(dirPath);

    await fs.mkdir(expandedPath, { recursive: true });
    return true;
  } catch (error) {
    console.error('Error creating directory:', error);
    return false;
  }
});

ipcMain.handle('list-files', async (_event, dirPath, extension) => {
  try {
    const expandedPath = expandBiorouterPath(dirPath);

    const files = await fs.readdir(expandedPath);
    if (extension) {
      return files.filter((file) => file.endsWith(extension));
    }
    return files;
  } catch (error) {
    if (
      typeof error === 'object' &&
      error !== null &&
      'code' in error &&
      (error.code === 'ENOTDIR' || error.code === 'ENOENT')
    ) {
      return [];
    }
    console.error('Error listing files:', error);
    return [];
  }
});

ipcMain.handle('delete-file', async (_event, filePath: string) => {
  try {
    const expandedPath = expandBiorouterPath(filePath);
    const resolvedPath = path.resolve(expandedPath);
    const allowedRoots = allowedFileRoots();
    const isAllowed = allowedRoots.some(
      (root) => resolvedPath.startsWith(root + path.sep) || resolvedPath === root
    );
    if (!isAllowed) {
      throw new Error(`Access denied: path '${resolvedPath}' is outside allowed directories`);
    }
    await fs.unlink(resolvedPath);
    return true;
  } catch (error) {
    console.error('Error deleting file:', error);
    return false;
  }
});

ipcMain.handle('list-skill-dirs', async (_event, dirPath: string) => {
  try {
    const expandedPath = expandBiorouterPath(dirPath);
    const entries = await fs.readdir(expandedPath, { withFileTypes: true });
    return entries.filter((e) => e.isDirectory()).map((e) => e.name);
  } catch {
    return [];
  }
});

ipcMain.handle('delete-directory', async (_event, dirPath: string) => {
  try {
    const expandedPath = expandBiorouterPath(dirPath);
    const resolvedPath = path.resolve(expandedPath);
    const allowedRoots = allowedFileRoots();
    const isAllowed = allowedRoots.some(
      (root) => resolvedPath.startsWith(root + path.sep) || resolvedPath === root
    );
    if (!isAllowed) {
      throw new Error(`Access denied: '${resolvedPath}' is outside allowed directories`);
    }
    await fs.rm(resolvedPath, { recursive: true, force: true });
    return true;
  } catch (error) {
    console.error('Error deleting directory:', error);
    return false;
  }
});

ipcMain.handle('show-message-box', async (_event, options) => {
  return dialog.showMessageBox(options);
});

ipcMain.handle('show-save-dialog', async (_event, options) => {
  return dialog.showSaveDialog(options);
});

ipcMain.handle(
  'save-diagnostics-bundle',
  async (event, sessionId: string, archive: DiagnosticsArchivePayload) => {
    try {
      if (!sessionId || typeof sessionId !== 'string') {
        throw new Error('A chat is required to generate diagnostics.');
      }

      const bytes = diagnosticsArchiveBytes(archive);
      const parent = BrowserWindow.fromWebContents(event.sender);
      const options = {
        title: 'Save Diagnostics Bundle',
        defaultPath: path.join(app.getPath('downloads'), diagnosticsArchiveFilename(sessionId)),
        buttonLabel: 'Save',
        filters: [{ name: 'ZIP Archives', extensions: ['zip'] }],
      };
      const result = parent
        ? await dialog.showSaveDialog(parent, options)
        : await dialog.showSaveDialog(options);

      if (result.canceled || !result.filePath) {
        return { canceled: true };
      }

      await fs.writeFile(result.filePath, bytes);
      return { canceled: false, filePath: result.filePath };
    } catch (error) {
      const message =
        error instanceof Error ? error.message : 'Failed to save the diagnostics bundle.';
      log.error('Failed to save diagnostics bundle:', error);
      return { canceled: false, error: message };
    }
  }
);

ipcMain.handle('get-allowed-extensions', async () => {
  return await getAllowList();
});

function parseFrontmatterFromSkillMd(
  content: string
): { name: string; description: string } | null {
  const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---(\r?\n|$)/);
  if (!match) return null;
  const fm = match[1];
  const nameMatch = fm.match(/^name:\s*([^\n]+)$/m);
  const descMatch = fm.match(/^description:\s*([^\n]+)$/m);
  if (!nameMatch?.[1]?.trim() || !descMatch?.[1]?.trim()) return null;
  return { name: nameMatch[1].trim(), description: descMatch[1].trim() };
}

// --- BAAM registry (Browse Skills / Browse Extensions) --------------------
// The marketplace catalog is published at biorouter.ucsf.edu/registry.json
// (generated from baam.html). We fetch it live so the in-app browser stays in
// sync with the website; the renderer ships a bundled snapshot as a fallback.
const REGISTRY_URL = 'https://biorouter.ucsf.edu/registry.json';

// Only these hosts may be fetched/downloaded from for the Browse feature. The
// registry's skill/extension assets all live on github.com or the site itself.
const REGISTRY_DOWNLOAD_HOSTS = new Set([
  'biorouter.ucsf.edu',
  'github.com',
  'objects.githubusercontent.com',
  'raw.githubusercontent.com',
  'codeload.github.com',
]);

function isAllowedRegistryUrl(rawUrl: string): URL | null {
  try {
    const parsed = new URL(rawUrl);
    if (parsed.protocol !== 'https:') return null;
    if (!REGISTRY_DOWNLOAD_HOSTS.has(parsed.hostname)) return null;
    return parsed;
  } catch {
    return null;
  }
}

/**
 * The last document that both fetched and parsed. Written on every success and
 * read on every failure, so an offline launch still shows the catalogue the
 * machine last saw rather than the snapshot frozen at build time — which is what
 * makes "an offline laptop can fail to learn a new private badge, but never
 * loses one" true across restarts rather than only within a session.
 */
function registryCachePath(): string {
  return path.join(app.getPath('userData'), 'registry-last-good.json');
}

// ⚠ The 10 s timeout, the validate-before-cache ordering and the stale replay
// all live in `utils/registryCache` — Electron-free on purpose. This file
// imports `electron` at the top level and therefore cannot be unit-tested, and
// the renderer's tests stop at the IPC boundary; with the composition inline
// here, an implementation that imported `registryCache` and never CALLED it
// passed every test this feature has, and the timeout was checked only by
// `grep -c AbortController`, which a comment satisfies. What is left below is
// exactly the part that needs Electron: the path (`app`) and the warning
// (`log`).

ipcMain.handle('registry:fetch', () =>
  fetchRegistryWithLastGood({
    url: REGISTRY_URL,
    cachePath: registryCachePath(),
    onWriteError: (err) => log.warn('Could not write the last-good registry cache:', err),
  })
);

// Download a registry asset (.zip skill bundle or .brxt extension) to a temp
// file and return its local path, for reuse by the existing install flows.
ipcMain.handle('registry:download', async (_event, { url }: { url: string }) => {
  const parsed = isAllowedRegistryUrl(url);
  if (!parsed) return { error: 'Refusing to download from an untrusted URL.' };

  const ext = parsed.pathname.toLowerCase().endsWith('.brxt') ? '.brxt' : '.zip';
  if (!parsed.pathname.toLowerCase().endsWith('.zip') && ext !== '.brxt') {
    return { error: 'Unsupported asset type.' };
  }

  try {
    const response = await fetch(url, {
      headers: { 'User-Agent': 'Biorouter' },
      redirect: 'follow',
    });
    if (!response.ok) return { error: `Download failed: HTTP ${response.status}` };

    const MAX_SIZE = 200 * 1024 * 1024; // 200MB ceiling
    const buf = Buffer.from(await response.arrayBuffer());
    if (buf.length > MAX_SIZE) return { error: 'Download too large.' };

    const dir = path.join(os.tmpdir(), 'biorouter-registry');
    fsSync.mkdirSync(dir, { recursive: true });
    const safeName = (path.basename(parsed.pathname) || `asset${ext}`).replace(
      /[^a-zA-Z0-9._-]/g,
      '_'
    );
    const dest = path.join(dir, `${crypto.randomBytes(6).toString('hex')}-${safeName}`);
    fsSync.writeFileSync(dest, buf);
    return { path: dest };
  } catch (err) {
    return { error: `Download failed: ${(err as Error).message}` };
  }
});

ipcMain.handle('brxt:open-file-dialog', async (event) => {
  // Allow automated tests to inject a file path without a native dialog
  if (process.env.PLAYWRIGHT_BRXT_FILE) {
    return process.env.PLAYWRIGHT_BRXT_FILE;
  }
  const win = BrowserWindow.fromWebContents(event.sender);
  const result = await dialog.showOpenDialog(win!, {
    title: 'Select Biorouter Extension Bundle',
    filters: [{ name: 'Biorouter Extension Bundle', extensions: ['brxt'] }],
    properties: ['openFile'],
  });
  if (result.canceled || result.filePaths.length === 0) return null;
  return result.filePaths[0];
});

ipcMain.handle('brxt:validate-and-read', async (_event, { filePath }: { filePath: string }) => {
  try {
    const zip = new AdmZip(filePath);
    const entries = zip.getEntries().map((e) => e.entryName);

    if (!entries.some((e) => e === 'manifest.json'))
      return { error: 'Missing manifest.json. This is not a valid .brxt bundle.' };
    if (!entries.some((e) => e.toLowerCase() === 'readme.md'))
      return { error: 'Missing README.md. This is not a valid .brxt bundle.' };
    if (!entries.some((e) => e === 'pyproject.toml'))
      return { error: 'Missing pyproject.toml. This is not a valid .brxt bundle.' };
    if (!entries.some((e) => e.startsWith('src/')))
      return { error: 'Missing src/ directory. This is not a valid .brxt bundle.' };

    const manifestEntry = zip.getEntry('manifest.json');
    if (!manifestEntry) return { error: 'Could not read manifest.json' };

    const manifest = JSON.parse(manifestEntry.getData().toString('utf8'));

    for (const field of [
      'name',
      'display_name',
      'description',
      'version',
      'entry_point',
      'repository',
    ]) {
      if (!manifest[field]) return { error: `manifest.json missing required field: "${field}"` };
    }

    if (!Array.isArray(manifest.env_vars))
      return { error: 'manifest.json "env_vars" must be an array' };

    // Scan for bundled skills in skills/<slug>/SKILL.md
    const skillsPreview: Array<{ slug: string; name: string; description: string }> = [];
    for (const entry of zip.getEntries()) {
      const m = entry.entryName.match(/^skills\/([^/]+)\/SKILL\.md$/);
      if (m) {
        const slug = m[1];
        const parsed = parseFrontmatterFromSkillMd(entry.getData().toString('utf8'));
        if (parsed)
          skillsPreview.push({ slug, name: parsed.name, description: parsed.description });
      }
    }

    return { manifest, skillsPreview };
  } catch (err) {
    return { error: `Failed to read bundle: ${(err as Error).message}` };
  }
});

// Generous cap: when a dependency has no prebuilt wheel, uv compiles it from
// source, which can take several minutes on its own.
const UV_SYNC_TIMEOUT_MS = 600_000;

/** Map well-known `uv sync` failure signatures to an actionable hint appended
 *  below the raw output. Checks run most-specific first. Mirrors
 *  `uv_sync_hint` in crates/biorouter-cli/src/commands/extension.rs. */
function uvSyncHint(detail: string): string | null {
  if (detail.includes('Symbol not found') && detail.includes('librustc_driver')) {
    // Homebrew rust links libLLVM.dylib dynamically and breaks when llvm is
    // upgraded; `brew upgrade rust` does not reliably fix it, so steer to the
    // self-contained rustup toolchain and removing the Homebrew one.
    return (
      'Your Homebrew Rust toolchain is broken. rustc aborts because Homebrew’s llvm was ' +
      'upgraded out from under it (a known Homebrew issue). `brew upgrade rust` usually ' +
      'does NOT fix this. Install the self-contained rustup toolchain and remove the ' +
      'Homebrew one so it takes priority:\n' +
      '    brew uninstall rust\n' +
      "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh\n" +
      'then fully restart Biorouter and retry.'
    );
  }
  if (detail.includes('cryptography') && cryptographyBuiltFromSource(detail)) {
    // cryptography ≥49 (2026-06-12) dropped x86_64 macOS wheels.
    return (
      '`cryptography` ≥49 no longer ships x86_64 (Intel) macOS wheels, so on an Intel Mac ' +
      'it must be compiled from source, which needs a Rust toolchain. Install rustup ' +
      '(https://rustup.rs) and retry, or ask the extension author to cap `cryptography<49` ' +
      '(the last series with Intel-Mac wheels).'
    );
  }
  if (detail.includes('maturin') || detail.includes('rustc')) {
    return (
      'A dependency has no prebuilt package for your platform, so it was compiled from ' +
      'source, which needs a working Rust toolchain. Install one via https://rustup.rs ' +
      '(or repair your existing install) and retry.'
    );
  }
  if (detail.includes('Failed to build')) {
    return (
      'A dependency has no prebuilt package for your platform, so uv tried to compile it ' +
      'from source. Make sure a compiler toolchain is installed, or ask the extension ' +
      'author to pin versions that ship prebuilt wheels.'
    );
  }
  return null;
}

/** True when stderr indicates `cryptography` was being built from source.
 *  Mirrors `cryptography_built_from_source` in the CLI crate. */
function cryptographyBuiltFromSource(detail: string): boolean {
  return (
    detail.includes('Failed to build `cryptography') ||
    detail.includes('Building cryptography') ||
    (detail.includes('cryptography') && detail.includes('maturin'))
  );
}

ipcMain.handle(
  'brxt:install',
  async (
    _event,
    {
      filePath,
      extensionName,
      registrySource,
    }: {
      filePath: string;
      extensionName: string;
      /**
       * Issue #56 Task 43 (DR-23). Present only for a marketplace install —
       * the BAAM registry `id` and the URL the bundle came from. A `.brxt`
       * dropped in by hand has neither, and correctly records nothing: the
       * daemon then falls back to the config-name join, which is the behaviour
       * that shipped before this task.
       */
      registrySource?: { registryId: string; sourceUrl?: string };
    }
  ) => {
    try {
      // ⚠ `extensionName` is the BUNDLE's own `manifest.name`, handed straight
      // through from `BrxtInstallModal.tsx` — so the archive names the
      // directory it is written into. Without this check a bundle declaring
      // `"name": "../../evil"` escapes the extensions root, and the desktop is
      // the third installer: the Rust transaction and `routes::shell` both
      // validate, and the `brxt:uninstall` handler below performs exactly this
      // check on exactly this string.
      if (
        !extensionName ||
        /[/\\]/.test(extensionName) ||
        extensionName === '..' ||
        extensionName === '.'
      ) {
        return { error: 'Invalid extension name.' };
      }
      // ⚠ Resolved, not hardcoded (#146). This handler creates a directory,
      // extracts an archive over it and runs `uv sync` in it; deriving the base
      // from `os.homedir()` meant a sandboxed dev build did all three inside
      // the developer's real extensions tree. `biorouterExtensionsDir` is the
      // one derivation in this process and the uninstall handler below reads
      // the same one, which is what makes the containment check meaningful.
      const extensionsBase = biorouterExtensionsDir();
      const installDir = path.join(extensionsBase, extensionName);
      if (!installDir.startsWith(extensionsBase + path.sep)) {
        return { error: 'Invalid extension name.' };
      }

      // Create install directory
      fsSync.mkdirSync(installDir, { recursive: true });

      // Extract bundle (zip-slip guarded: rejects entries that escape installDir)
      const zip = new AdmZip(filePath);
      safeExtractZip(zip, installDir);

      // Pre-build the virtual environment.
      // Async, not spawnSync: UV_SYNC_TIMEOUT_MS is ten minutes, and a
      // synchronous child here froze the entire app for the whole build (#88).
      const uvResult = await runProbe('uv', ['sync'], UV_SYNC_TIMEOUT_MS, { cwd: installDir });

      if (!uvResult.ok) {
        if (uvResult.timedOut) {
          throw new Error(
            `uv sync timed out after ${UV_SYNC_TIMEOUT_MS / 60_000} minutes. ` +
              'A dependency may be compiling from source on a slow connection or machine. ' +
              'Try again, or build manually with `uv sync` in ' +
              installDir
          );
        }
        const detail =
          uvResult.stderr ||
          uvResult.stdout ||
          uvResult.error ||
          `exited with status ${uvResult.code}`;
        const hint = uvSyncHint(detail);
        throw new Error(`uv sync failed: ${detail}${hint ? `\n\nHint: ${hint}` : ''}`);
      }

      // Issue #56 Task 43 (DR-23). AFTER the bundle is on disk and its venv
      // built, because a record for an install that then failed would claim a
      // provenance no config entry has. Never fatal: losing the record costs
      // the rename protection, whereas failing here costs the user the
      // extension they just installed.
      if (registrySource?.registryId) {
        const recorded = recordExtensionProvenance({
          extensionName,
          registryId: registrySource.registryId,
          // The rename-proof half: `installDir` is what the stdio config's
          // `--directory` argument will point at, and a later rename of the
          // config entry cannot move it without breaking the extension.
          installDir,
          sourceUrl: registrySource.sourceUrl,
          bundlePath: filePath,
        });
        if (!recorded) {
          log.warn(
            `[brxt] could not record provenance for ${extensionName}; its privacy tier will ` +
              `fall back to the config-name join and a local rename would lose it`
          );
        }
      }

      return { success: true, installDir };
    } catch (err) {
      return { error: `Installation failed: ${(err as Error).message}` };
    }
  }
);

ipcMain.handle('brxt:uninstall', async (_event, { extensionName }: { extensionName: string }) => {
  try {
    if (
      !extensionName ||
      /[/\\]/.test(extensionName) ||
      extensionName === '..' ||
      extensionName === '.'
    ) {
      return { error: 'Invalid extension name.' };
    }
    // ⚠ The worst of the #146 sites, and the reason there is now exactly one
    // resolver: `installDir` is handed straight to a recursive, forced
    // `rmSync`. Derived from `os.homedir()`, a sandboxed dev build deleted out
    // of the developer's REAL extensions tree. Resolved once and used for both
    // the target and the containment base, so the two can never disagree.
    const extensionsBase = biorouterExtensionsDir();
    const installDir = path.join(extensionsBase, extensionName);
    if (!installDir.startsWith(extensionsBase + path.sep)) {
      return { error: 'Invalid extension name.' };
    }
    if (fsSync.existsSync(installDir)) {
      fsSync.rmSync(installDir, { recursive: true, force: true });
    }
    return { success: true as const };
  } catch (err) {
    return { error: `Uninstall failed: ${(err as Error).message}` };
  }
});

function handleBrxtFileOpen(filePath: string) {
  // Find the main window (or store for when one is ready)
  const win = BrowserWindow.getAllWindows().find((w) => !w.isDestroyed());
  if (win) {
    win.webContents.send('open-brxt-file', filePath);
    win.focus();
  } else {
    // Store for when window is ready
    pendingBrxtFilePath = filePath;
  }
}

/**
 * IPC for the "Install Biorouter CLI" affordance. The actual install logic
 * lives in the bundled CLI (`biorouter setup-path`) so the terminal and the
 * desktop app share one implementation (Rust `biorouter::system::install_cli`).
 */
// Run `<binary> --version` and return the parsed dotted version, or null if it
// can't be determined (missing binary, broken symlink, non-zero exit). The CLI
// prints just the version (e.g. " 1.85.0") thanks to its empty display name.
async function cliVersionOf(binary: string): Promise<string | null> {
  // Async, not spawnSync: `cli:status` is invoked from the renderer on every
  // launch, and a synchronous child here froze the main thread for as long as
  // the binary took to answer (see #88).
  const res = await runProbe(binary, ['--version'], 10_000);
  if (!res.ok) return null;
  const m = (res.stdout || res.stderr || '').match(/(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.]+)?)/);
  return m ? m[1] : null;
}

// True if dotted version `a` is strictly older than `b` (segment-wise numeric;
// mirrors the Rust `version_newer` used by `biorouter::system`).
function isVersionOlder(a: string, b: string): boolean {
  const parse = (s: string) =>
    s
      .replace(/^v/, '')
      .split(/[.\-+]/)
      .map((p) => parseInt(p, 10) || 0);
  const va = parse(a);
  const vb = parse(b);
  for (let i = 0; i < Math.max(va.length, vb.length); i++) {
    const x = va[i] ?? 0;
    const y = vb[i] ?? 0;
    if (x !== y) return x < y;
  }
  return false;
}

function usableWorkingDir(workingDir?: string): string | undefined {
  if (!workingDir || typeof workingDir !== 'string') return undefined;
  try {
    if (fsSync.statSync(workingDir).isDirectory()) {
      return workingDir;
    }
  } catch {
    return undefined;
  }
  return undefined;
}

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

type TerminalBackend = 'pty' | 'process';

type TerminalCreateOptions = {
  workingDir?: string;
  cols?: number;
  rows?: number;
};

type TerminalSession = RegisteredTerminalSession & {
  backend: TerminalBackend;
  cwd: string;
  write: (data: string) => void;
  resize: (cols: number, rows: number) => void;
};

type NodePtyModule = typeof import('node-pty');

const terminalSessions = new TerminalSessionRegistry<TerminalSession>((error) =>
  log.warn('[terminal] failed to dispose session:', error)
);
let nodePtyModule: NodePtyModule | null | undefined;

/**
 * Free every shell a renderer owns once its document goes away.
 *
 * `destroyed` alone was not enough. A reload — Cmd+R (wired in
 * createChat), View > Reload, the `reload-app` IPC — swaps the document while
 * the webContents lives on, so React never runs its effect cleanups and the
 * renderer's `terminal:dispose` calls never arrive. Those shells then held slots
 * that no UI could reach, and the session cap fired with nothing visibly open.
 *
 * `isSameDocument` is the load-bearing filter: the app is a hash router, so
 * ordinary in-app navigation (`#/pair` -> `#/settings`) fires this event too and
 * must NOT kill the user's terminals.
 */
function registerTerminalOwnerTeardown(contents: Electron.WebContents) {
  if (terminalOwnerTeardownRegistered.has(contents.id)) return;
  terminalOwnerTeardownRegistered.add(contents.id);

  const release = (reason: string) => {
    const released = terminalSessions.releaseOwner(contents.id);
    if (released > 0) {
      log.info(
        `[terminal] released ${released} session(s) for webContents ${contents.id}:`,
        reason
      );
    }
  };

  contents.on('did-start-navigation', (details) => {
    if (!details.isMainFrame || details.isSameDocument) return;
    // `did-start-navigation` fires when the NavigationRequest is created, which
    // is BEFORE navigation throttles run — and Electron implements
    // `will-navigate` as a throttle. So a navigation that
    // `blockOffOriginNavigation` is about to cancel still reaches this listener,
    // and releasing here would kill every shell in the window for a document
    // that never actually changes (with no did-fail-load compensation to undo
    // it). Every navigation that CAN commit here is same-origin — Cmd+R, View >
    // Reload, the `reload-app` IPC, loadURL of the renderer entry — so gating on
    // the app origin keeps the reload teardown this exists for while making a
    // cancelled navigation a no-op.
    if (!isAppOrigin(details.url, rendererEntryUrl())) return;
    release('document replaced');
  });
  contents.on('render-process-gone', () => release('render process gone'));
  contents.once('destroyed', () => {
    release('webContents destroyed');
    terminalOwnerTeardownRegistered.delete(contents.id);
  });
}

const terminalOwnerTeardownRegistered = new Set<number>();

function terminalSize(value: unknown, fallback: number, min: number, max: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, Math.floor(value)));
}

function terminalShell(): { shellPath: string; ptyArgs: string[]; processArgs: string[] } {
  if (process.platform === 'win32') {
    return {
      shellPath: process.env.ComSpec || 'cmd.exe',
      ptyArgs: [],
      processArgs: [],
    };
  }
  const shellPath = process.env.SHELL || '/bin/zsh';
  return {
    shellPath,
    ptyArgs: shellPath.endsWith('zsh') ? ['-l'] : [],
    processArgs: shellPath.endsWith('zsh') ? ['-i'] : [],
  };
}

function terminalEnv(): Record<string, string | undefined> {
  return {
    ...SPAWN_ENV,
    COLORTERM: 'truecolor',
    FORCE_COLOR: '1',
    TERM: 'xterm-256color',
  };
}

// node-pty's npm tarball ships `prebuilds/<platform>-<arch>/spawn-helper` with
// mode 0644, and nothing in its install ever chmods it (its `install` script
// short-circuits on the presence of prebuilds, so node-gyp — which would build
// and chmod the helper — never runs). On macOS `pty.fork()` `posix_spawn`s that
// helper, so a non-executable one makes every terminal die with the opaque
// native error "posix_spawnp failed."
//
// `scripts/fix-node-pty-permissions.mjs` repairs it on postinstall and again at
// package time, which covers dev trees and shipped builds. This is the last
// line of defence for a dev tree whose node_modules was installed with
// --ignore-scripts or restored from an archive: repairing costs one stat.
//
// Deliberately dev-only. Inside a packaged app the helper lives in a signed —
// and, on macOS, notarized — bundle; writing to it at runtime is exactly the
// kind of self-modification that bundle validation exists to catch, and the
// package-time repair has already run. There we only report.
function ensureSpawnHelperExecutable(): void {
  if (process.platform === 'win32') return; // ConPTY/winpty; no spawn-helper
  // In dev this is the project directory; in a packaged app it is `app.asar`,
  // and the helper is the copy under `app.asar.unpacked` — the same rewrite
  // node-pty's unixTerminal.js applies when it builds the path it spawns.
  const nodePtyRoot = path.join(
    app.getAppPath().replace('app.asar', 'app.asar.unpacked'),
    'node_modules',
    'node-pty'
  );
  const prebuild = path.join(
    nodePtyRoot,
    'prebuilds',
    `${process.platform}-${process.arch}`,
    'spawn-helper'
  );
  for (const helper of [prebuild, path.join(nodePtyRoot, 'build', 'Release', 'spawn-helper')]) {
    let mode: number;
    try {
      mode = fsSync.statSync(helper).mode;
    } catch {
      continue;
    }
    if ((mode & 0o111) === 0o111) continue;
    if (app.isPackaged) {
      log.error(
        `[terminal] ${helper} is not executable (mode ${(mode & 0o7777).toString(8)}). ` +
          'Every pty spawn will fail with "posix_spawnp failed." This build was packaged ' +
          'from a node_modules tree that scripts/fix-node-pty-permissions.mjs never ran against.'
      );
      continue;
    }
    try {
      fsSync.chmodSync(helper, (mode & 0o7777) | 0o755);
      log.warn(`[terminal] restored the executable bit on ${helper}`);
    } catch (error) {
      log.error(`[terminal] could not make ${helper} executable:`, error);
    }
  }
}

async function loadNodePty(): Promise<NodePtyModule | null> {
  if (nodePtyModule !== undefined) return nodePtyModule;
  try {
    nodePtyModule = await import('node-pty');
    ensureSpawnHelperExecutable();
  } catch (error) {
    // In a packaged app this is a packaging regression, not a missing optional
    // feature: node-pty is a declared dependency that forge.config.ts keeps out
    // of `packagerConfig.ignore` precisely so it ships. The pipe fallback below
    // has no TTY — no prompt, no line editing, no job control, `isatty()` false
    // — so it looks like a broken terminal rather than a broken build. Log it
    // loudly enough that the next person greps for it.
    const level = app.isPackaged ? 'error' : 'warn';
    log[level]('[terminal] node-pty unavailable; falling back to process pipes:', error);
    nodePtyModule = null;
  }
  return nodePtyModule;
}

function disposeTerminalSession(sessionId: string) {
  return terminalSessions.release(sessionId);
}

function registerCliInstallHandlers() {
  // Is the `biorouter` command callable from a terminal, and is it current?
  //
  // Reports both the bundled version (what this app ships) and the on-PATH
  // version (what `biorouter` resolves to in a terminal). They can differ:
  //   • macOS/Linux GUI installs symlink into the app bundle, so they usually
  //     auto-upgrade when the app is replaced in place;
  //   • Windows installs *copy* the binary, so they go stale on upgrade;
  //   • a standalone .deb/.rpm CLI is a real binary in /usr/bin that a GUI
  //     upgrade can never touch;
  //   • a symlink can dangle if the old app bundle was moved/removed.
  // `needsUpdate` is what the renderer uses to offer an upgrade in all of these
  // cases — re-running the installer (`cli:install`) overwrites the entry.
  ipcMain.handle('cli:status', async () => {
    let bundled: string | null = null;
    try {
      bundled = getBiorouterCliBinaryPath(app);
    } catch (e) {
      log.warn('[cli:status] bundled CLI not found:', (e as Error).message);
      bundled = null;
    }
    const probe = await runProbe(process.platform === 'win32' ? 'where' : 'which', ['biorouter']);
    const pathLocation =
      probe.ok && probe.stdout.trim().length > 0
        ? probe.stdout.trim().split(/\r?\n/)[0].trim()
        : null;

    // Both version probes are independent subprocesses — resolve them together
    // rather than paying for one and then the other.
    const [bundledVersion, pathVersion] = await Promise.all([
      bundled ? cliVersionOf(bundled) : Promise.resolve(null),
      // Resolve the on-PATH binary's version. A dangling symlink / broken binary
      // yields null here even though `which` found a name — treat that as "not
      // really installed" so the user is still offered the install.
      pathLocation ? cliVersionOf('biorouter') : Promise.resolve(null),
    ]);
    const onPath = pathVersion !== null;

    const needsUpdate =
      onPath &&
      bundledVersion !== null &&
      pathVersion !== null &&
      isVersionOlder(pathVersion, bundledVersion);

    return {
      bundled,
      onPath,
      pathLocation,
      bundledVersion,
      pathVersion,
      needsUpdate,
      // `which` found a name but it won't run — a broken/dangling install.
      brokenOnPath: pathLocation !== null && pathVersion === null,
    };
  });

  ipcMain.handle('terminal:create', async (event, options?: TerminalCreateOptions) => {
    const cwd = usableWorkingDir(options?.workingDir) || os.homedir();
    const cols = terminalSize(options?.cols, 80, 24, 500);
    const rows = terminalSize(options?.rows, 18, 8, 200);
    const { shellPath, ptyArgs, processArgs } = terminalShell();
    const sessionId = crypto.randomUUID();
    const owner = event.sender;
    let didExit = false;

    // Loaded BEFORE the teardown registration, not inside the try below, so the
    // span from "this window's shells can be released" to "this session is in
    // the registry" contains no await. Awaiting in that span would let a reload
    // fire `releaseOwner` while this session is not yet registered, and the pty
    // would then spawn and register under a dead document that can never send
    // `terminal:dispose` for it. `loadNodePty` resolves to null rather than
    // rejecting, so hoisting it out of the try changes no error handling.
    const pty = await loadNodePty();

    // Reload / navigate / crash all free this window's shells. Registered per
    // webContents, idempotently, so it survives the renderer being replaced.
    registerTerminalOwnerTeardown(owner);

    const registerSession = (
      session: Omit<TerminalSession, 'removeOwnerDestroyedListener'>
    ): void => {
      const handleOwnerDestroyed = () => disposeTerminalSession(sessionId);
      owner.once('destroyed', handleOwnerDestroyed);
      terminalSessions.add(sessionId, {
        ...session,
        removeOwnerDestroyedListener: () => {
          owner.removeListener('destroyed', handleOwnerDestroyed);
        },
      });
    };

    const sendData = (data: string) => {
      if (!owner.isDestroyed()) {
        owner.send('terminal:data', { sessionId, data });
      }
    };
    const sendExit = (exitCode: number | null, signal?: string | number | null) => {
      if (didExit) return;
      didExit = true;
      const session = terminalSessions.forget(sessionId);
      session?.removeOwnerDestroyedListener();
      if (!owner.isDestroyed()) {
        owner.send('terminal:exit', {
          sessionId,
          exitCode,
          signal: signal === null || typeof signal === 'undefined' ? null : String(signal),
        });
      }
    };

    try {
      const sessionLimit = maxTerminalSessionsPerOwner();
      if (terminalSessions.countForOwner(owner.id) >= sessionLimit) {
        return { success: false, error: terminalSessionLimitMessage(sessionLimit) };
      }
      if (pty) {
        const ptyProcess = pty.spawn(shellPath, ptyArgs, {
          cols,
          cwd,
          env: terminalEnv(),
          name: 'xterm-256color',
          rows,
        });
        const dataDisposer = ptyProcess.onData(sendData);
        const exitDisposer = ptyProcess.onExit(({ exitCode, signal }) => {
          dataDisposer.dispose();
          exitDisposer.dispose();
          sendExit(exitCode, signal);
        });
        registerSession({
          backend: 'pty',
          cwd,
          ownerId: owner.id,
          write: (data) => ptyProcess.write(data),
          resize: (nextCols, nextRows) => {
            ptyProcess.resize(
              terminalSize(nextCols, cols, 24, 500),
              terminalSize(nextRows, rows, 8, 200)
            );
          },
          dispose: () => {
            didExit = true;
            dataDisposer.dispose();
            exitDisposer.dispose();
            ptyProcess.kill();
          },
        });
        return { success: true, sessionId, cwd, backend: 'pty' as const };
      }

      const child = spawn(shellPath, processArgs, {
        cwd,
        env: terminalEnv(),
        stdio: 'pipe',
        windowsHide: true,
      });
      child.stdout.setEncoding('utf8');
      child.stderr.setEncoding('utf8');
      const handleStdout = (data: string) => sendData(data);
      const handleStderr = (data: string) => sendData(data);
      const handleClose = (code: number | null, signal: string | null) => sendExit(code, signal);
      const handleError = (error: Error) => {
        sendData(`\r\n${error.message}\r\n`);
        sendExit(1, null);
      };
      child.stdout.on('data', handleStdout);
      child.stderr.on('data', handleStderr);
      child.on('close', handleClose);
      child.on('error', handleError);
      registerSession({
        backend: 'process',
        cwd,
        ownerId: owner.id,
        write: (data) => {
          child.stdin.write(data);
        },
        resize: () => {},
        dispose: () => {
          didExit = true;
          child.stdout.removeListener('data', handleStdout);
          child.stderr.removeListener('data', handleStderr);
          child.removeListener('close', handleClose);
          child.removeListener('error', handleError);
          child.kill();
        },
      });
      return { success: true, sessionId, cwd, backend: 'process' as const };
    } catch (error) {
      return { success: false, error: (error as Error).message };
    }
  });

  // A session may only be driven by the renderer that created it. Without this,
  // any window holding a session id could write into (or kill) another window's
  // shell.
  const ownedTerminalSession = (event: Electron.IpcMainInvokeEvent, sessionId: string) =>
    terminalSessions.getOwned(sessionId, event.sender.id) ?? null;

  ipcMain.handle('terminal:write', async (event, sessionId: string, data: string) => {
    const session = ownedTerminalSession(event, sessionId);
    if (!session) return { success: false, error: 'This terminal is no longer running.' };
    session.write(data);
    return { success: true };
  });

  ipcMain.handle(
    'terminal:resize',
    async (event, sessionId: string, cols: number, rows: number) => {
      const session = ownedTerminalSession(event, sessionId);
      if (!session) return { success: false, error: 'This terminal is no longer running.' };
      session.resize(cols, rows);
      return { success: true };
    }
  );

  ipcMain.handle('terminal:dispose', async (event, sessionId: string) => {
    if (!ownedTerminalSession(event, sessionId)) {
      return { success: false, error: 'This terminal is no longer running.' };
    }
    disposeTerminalSession(sessionId);
    return { success: true };
  });

  // Launch the installed CLI in the user's terminal app. Assumes `biorouter`
  // is already on PATH (the renderer checks `cli:status` first and offers the
  // install flow otherwise).
  ipcMain.handle('cli:launch', async (_event, workingDir?: string) => {
    // Launch the CLI in `workingDir` when supplied (the chat's working
    // directory) so the terminal opens in the exact folder the user is
    // working in, rather than the terminal's default/home directory. Only
    // honor an existing directory; fall back to no `cd` otherwise.
    const cwd = usableWorkingDir(workingDir);
    try {
      if (process.platform === 'darwin') {
        // Open Terminal.app with `do script`, which runs the literal `biorouter`
        // command in a new window (prefixed with a `cd` into the working
        // directory). This is transparent — the user sees `biorouter` run, not a
        // generated helper script — and relies on the CLI already being on PATH.
        const doScript = cwd ? `cd ${shellQuote(cwd)} && biorouter` : 'biorouter';
        // Escape for the AppleScript string literal: backslashes first, then
        // double quotes.
        const asLiteral = doScript.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
        const res = await runProbe(
          'osascript',
          [
            '-e',
            `tell application "Terminal" to do script "${asLiteral}"`,
            '-e',
            'tell application "Terminal" to activate',
          ],
          15_000
        );
        if (!res.ok) {
          return { success: false, error: (res.stderr || 'Failed to open Terminal').trim() };
        }
        return { success: true };
      }

      if (process.platform === 'win32') {
        // `start` opens a new console window that keeps running the CLI.
        // `/d <dir>` sets that window's starting directory.
        const startArgs = ['/c', 'start', 'Biorouter CLI'];
        if (cwd) {
          startArgs.push('/d', cwd);
        }
        startArgs.push('cmd', '/k', 'biorouter');
        const child = spawn('cmd.exe', startArgs, {
          env: SPAWN_ENV,
          detached: true,
          stdio: 'ignore',
        });
        child.unref();
        return { success: true };
      }

      // Linux: walk the common terminal emulators and use the first available.
      const candidates: [string, string[]][] = [
        ['x-terminal-emulator', ['-e', 'biorouter']],
        ['gnome-terminal', ['--', 'biorouter']],
        ['konsole', ['-e', 'biorouter']],
        ['xfce4-terminal', ['-e', 'biorouter']],
        ['kitty', ['biorouter']],
        ['alacritty', ['-e', 'biorouter']],
        ['xterm', ['-e', 'biorouter']],
      ];
      for (const [term, args] of candidates) {
        const found = await runProbe('which', [term]);
        if (found.ok && found.stdout.trim()) {
          const child = spawn(term, args, {
            env: SPAWN_ENV,
            detached: true,
            stdio: 'ignore',
            ...(cwd ? { cwd } : {}),
          });
          child.unref();
          return { success: true };
        }
      }
      return {
        success: false,
        error: 'No terminal emulator found. Run `biorouter` from your terminal instead.',
      };
    } catch (e) {
      return { success: false, error: (e as Error).message };
    }
  });

  // Install the bundled CLI onto PATH by delegating to `biorouter setup-path`.
  ipcMain.handle('cli:install', async () => {
    let cli: string;
    try {
      cli = getBiorouterCliBinaryPath(app);
    } catch (e) {
      return { success: false, error: `Bundled CLI not found: ${(e as Error).message}` };
    }
    const res = await runProbe(cli, ['setup-path'], 60_000);
    if (res.ok) {
      // The CLI just changed what's on PATH; a stale cached probe would report
      // the pre-install state back to the modal's verification step.
      invalidateDependencyCache();
      return { success: true, output: res.stdout.trim() };
    }
    return {
      success: false,
      error: (res.stderr || res.stdout || `setup-path exited with ${res.code}`).trim(),
      command: `${cli} setup-path`,
    };
  });
}

const createNewWindow = async (app: App, dir?: string | null) => {
  const recentDirs = loadRecentDirs();
  const openDir = dir || (recentDirs.length > 0 ? recentDirs[0] : undefined);
  return await createChat(app, undefined, openDir);
};

const focusWindow = () => {
  const windows = BrowserWindow.getAllWindows();
  if (windows.length > 0) {
    windows.forEach((win) => {
      win.show();
    });
    windows[windows.length - 1].webContents.send('focus-input');
  } else {
    createNewWindow(app);
  }
};

/**
 * "New Chat" — Cmd+T, the browser's new-tab key.
 *
 * It must be a menu item, not a renderer keydown listener, and this is not a
 * style preference: an Electron menu accelerator is consumed by the menu before
 * the web contents ever sees the key, so a listener in the renderer could never
 * have won. Cmd+T was already claimed here (it sent `set-view ''`, which merely
 * navigated Home), which is exactly the trap `role: 'close'` set for Cmd+W —
 * a key silently owned by the menu, with the renderer helpless.
 *
 * The renderer decides what "new tab" means (chatGroups' newTabRegistry); if it
 * has no tab surface mounted it navigates to /pair and opens one there, so
 * Cmd+T works from Settings exactly as it does in a browser from any page.
 *
 * Prefer the window Electron hands the click over getFocusedWindow(), for the
 * reason the Close Tab item documents: the accelerator fires FOR a window, and
 * getFocusedWindow() returns null often enough (e.g. under an automation
 * driver) that relying on it alone makes the key silently do nothing.
 */
function newChatTabItem(label: string, accelerator?: string): MenuItemConstructorOptions {
  return {
    label,
    ...(accelerator ? { accelerator } : {}),
    click(_item, browserWindow) {
      const target =
        browserWindow instanceof BrowserWindow ? browserWindow : BrowserWindow.getFocusedWindow();
      target?.webContents.send('new-chat-tab');
    },
  };
}

function buildApplicationMenu() {
  const isMac = process.platform === 'darwin';

  // Find submenu — inserted into Edit after Select All (roles don't allow inline custom items)
  const findSubmenu: MenuItemConstructorOptions[] = [
    {
      label: 'Find…',
      accelerator: isMac ? 'Command+F' : 'Control+F',
      click() {
        BrowserWindow.getFocusedWindow()?.webContents.send('find-command');
      },
    },
    {
      label: 'Find Next',
      accelerator: isMac ? 'Command+G' : 'Control+G',
      click() {
        BrowserWindow.getFocusedWindow()?.webContents.send('find-next');
      },
    },
    {
      label: 'Find Previous',
      accelerator: isMac ? 'Shift+Command+G' : 'Shift+Control+G',
      click() {
        BrowserWindow.getFocusedWindow()?.webContents.send('find-previous');
      },
    },
    ...(isMac
      ? [
          {
            label: 'Use Selection for Find',
            accelerator: 'Command+E',
            click() {
              BrowserWindow.getFocusedWindow()?.webContents.send('use-selection-find');
            },
          } as MenuItemConstructorOptions,
        ]
      : []),
  ];

  const template: MenuItemConstructorOptions[] = [
    // ── Biorouter app menu (macOS only) ──────────────────────────────────
    ...(isMac
      ? [
          {
            label: 'Biorouter',
            submenu: [
              { role: 'about' as const },
              { type: 'separator' as const },
              {
                label: 'Settings',
                accelerator: 'CmdOrCtrl+,',
                click() {
                  BrowserWindow.getFocusedWindow()?.webContents.send('set-view', 'settings');
                },
              },
              { type: 'separator' as const },
              {
                label: 'Check for Updates…',
                click: openUpdateSettings,
              },
              {
                label: 'Check for Dependencies…',
                click() {
                  triggerDependencyCheck();
                },
              },
              {
                label: 'Check for Extension Updates',
                click() {
                  runExtensionUpdateCheck();
                },
              },
              { type: 'separator' as const },
              { role: 'quit' as const, label: 'Quit Biorouter' },
            ],
          } as MenuItemConstructorOptions,
        ]
      : []),

    // ── Go ────────────────────────────────────────────────────────────────
    {
      label: 'Go',
      submenu: [
        {
          label: 'Home',
          accelerator: 'CmdOrCtrl+1',
          click() {
            BrowserWindow.getFocusedWindow()?.webContents.send('set-view', '');
          },
        },
        newChatTabItem('New Chat', 'CmdOrCtrl+T'),
        {
          label: 'History',
          accelerator: 'CmdOrCtrl+2',
          click() {
            BrowserWindow.getFocusedWindow()?.webContents.send('set-view', 'sessions');
          },
        },
        { type: 'separator' as const },
        {
          label: 'Workflows',
          accelerator: 'CmdOrCtrl+3',
          click() {
            BrowserWindow.getFocusedWindow()?.webContents.send('set-view', 'workflows');
          },
        },
        {
          label: 'Scheduler',
          accelerator: 'CmdOrCtrl+4',
          click() {
            BrowserWindow.getFocusedWindow()?.webContents.send('set-view', 'schedules');
          },
        },
        { type: 'separator' as const },
        {
          label: 'Extensions',
          accelerator: 'CmdOrCtrl+5',
          click() {
            BrowserWindow.getFocusedWindow()?.webContents.send('set-view', 'extensions');
          },
        },
        {
          label: 'Skills',
          accelerator: 'CmdOrCtrl+6',
          click() {
            BrowserWindow.getFocusedWindow()?.webContents.send('set-view', 'skills');
          },
        },
      ],
    },

    // ── File ─────────────────────────────────────────────────────────────
    {
      label: 'File',
      submenu: [
        // Same item, no accelerator — Go owns the key, File carries the
        // discoverable duplicate. Both must do the same thing or the menu lies.
        newChatTabItem('New Chat'),
        {
          label: 'New Window',
          accelerator: isMac ? 'Cmd+N' : 'Ctrl+N',
          click() {
            // ⚠ Call the function, do NOT `ipcMain.emit('create-chat-window')`.
            //
            // `ipcMain` is a plain EventEmitter, so a bare `emit` invokes the
            // listener with `event === undefined`. Since #78 that listener
            // anchors the new window on `event.sender`, so the bare emit threw
            // a TypeError inside an async listener: an unhandled rejection, no
            // window, and nothing on screen to say why. Cmd+N did nothing at
            // all, and the dock menu and titlebar control kept working, which
            // is presumably why it went unnoticed.
            //
            // A menu click has no renderer sender to anchor on by nature, so
            // the IPC handler is the wrong door for it. This is the same call
            // the dock menu makes.
            void createNewWindow(app);
          },
        },
        { type: 'separator' as const },
        {
          label: 'Open Directory…',
          accelerator: 'CmdOrCtrl+O',
          click: () => openDirectoryDialog(),
        },
        ...(() => {
          const recentFiles = buildRecentFilesMenu();
          return recentFiles.length > 0
            ? [{ label: 'Recent Directories', submenu: recentFiles } as MenuItemConstructorOptions]
            : [];
        })(),
        { type: 'separator' as const },
        // Cmd+W closes the TAB, Shift+Cmd+W closes the window — Safari/Chrome's
        // split, and now ours, because /pair is a tabbed surface.
        //
        // This item must exist. `role: 'close'` silently claims CmdOrCtrl+W as
        // its default accelerator, and a menu accelerator is consumed before the
        // renderer sees the keydown — so no amount of renderer-side key handling
        // could have closed a tab instead. It would have closed the window and
        // every tab in it. The renderer decides what to do (chatGroups'
        // closeActiveTabRegistry); if it has no tab to close it calls
        // 'close-window' itself, so a tabless route still closes on Cmd+W.
        {
          label: 'Close Tab',
          accelerator: 'CmdOrCtrl+W',
          // Prefer the window Electron hands the click over getFocusedWindow():
          // the accelerator fires FOR a window, and that window is the honest
          // target. getFocusedWindow() is only the fallback (and it returns null
          // often enough — e.g. under an automation driver — that relying on it
          // alone makes Cmd+W silently do nothing).
          click(_item, browserWindow) {
            const target =
              browserWindow instanceof BrowserWindow
                ? browserWindow
                : BrowserWindow.getFocusedWindow();
            target?.webContents.send('close-active-tab');
          },
        },
        { role: 'close' as const, label: 'Close Window', accelerator: 'Shift+CmdOrCtrl+W' },
        {
          label: 'Focus Biorouter Window',
          accelerator: 'CmdOrCtrl+Alt+G',
          click() {
            focusWindow();
          },
        },
      ],
    },

    // ── Edit (standard roles + Find inserted after build) ─────────────
    { role: 'editMenu' as const },

    // ── Extensions ───────────────────────────────────────────────────────
    {
      label: 'Extensions',
      submenu: [
        {
          label: 'Install Extension (.brxt)',
          click() {
            BrowserWindow.getFocusedWindow()?.webContents.send('set-view', 'extensions');
          },
        },
        {
          label: 'Browse Extensions',
          click() {
            shell.openExternal('http://biorouter.ucsf.edu/baam');
          },
        },
        {
          label: 'Add Custom Extension…',
          click() {
            BrowserWindow.getFocusedWindow()?.webContents.send('set-view', 'extensions');
          },
        },
        { type: 'separator' as const },
        {
          label: 'Check for Extension Updates',
          click() {
            runExtensionUpdateCheck();
          },
        },
      ],
    },

    // ── Providers ────────────────────────────────────────────────────────
    {
      label: 'Providers',
      submenu: [
        {
          label: 'Configure Providers…',
          click() {
            BrowserWindow.getFocusedWindow()?.webContents.send('set-view', 'configure-providers');
          },
        },
        {
          label: 'Switch Model…',
          click() {
            BrowserWindow.getFocusedWindow()?.webContents.send('set-view', 'settings', 'models');
          },
        },
        {
          label: 'Reset Provider',
          click() {
            BrowserWindow.getFocusedWindow()?.webContents.send('set-view', 'configure-providers');
          },
        },
      ],
    },

    // ── View (theme toggles + standard Electron view roles) ──────────────
    {
      label: 'View',
      submenu: [
        {
          label: 'Light Mode',
          click() {
            BrowserWindow.getAllWindows().forEach((w) =>
              w.webContents.send('theme-changed', { theme: 'light', useSystemTheme: false })
            );
          },
        },
        {
          label: 'Dark Mode',
          click() {
            BrowserWindow.getAllWindows().forEach((w) =>
              w.webContents.send('theme-changed', { theme: 'dark', useSystemTheme: false })
            );
          },
        },
        {
          label: 'System Mode',
          click() {
            // useSystemTheme: true — ThemeContext reads OS preference and ignores the theme field
            BrowserWindow.getAllWindows().forEach((w) =>
              w.webContents.send('theme-changed', { theme: 'light', useSystemTheme: true })
            );
          },
        },
        { type: 'separator' as const },
        { role: 'reload' as const },
        { role: 'forceReload' as const },
        { role: 'toggleDevTools' as const },
        { type: 'separator' as const },
        { role: 'resetZoom' as const },
        { role: 'zoomIn' as const },
        { role: 'zoomOut' as const },
        { type: 'separator' as const },
        { role: 'togglefullscreen' as const },
      ],
    },

    // ── Help ─────────────────────────────────────────────────────────────
    {
      label: 'Help',
      submenu: [
        {
          label: 'Biorouter Documentation',
          click() {
            shell.openExternal('http://biorouter.ucsf.edu/docs');
          },
        },
        { type: 'separator' as const },
        {
          label: 'Report a Bug…',
          click() {
            shell.openExternal(
              'https://github.com/BaranziniLab/biorouter/issues/new?template=bug_report.md'
            );
          },
        },
        {
          label: 'Request a Feature…',
          click() {
            shell.openExternal(
              'https://github.com/BaranziniLab/biorouter/issues/new?template=feature_request.md'
            );
          },
        },
        { type: 'separator' as const },
        { label: `v${version || app.getVersion()}`, enabled: false },
      ],
    },

    // ── Window (standard roles; Always on Top added after build) ─────────
    { role: 'windowMenu' as const },
  ];

  const menu = Menu.buildFromTemplate(template);

  // Insert Find submenu into Edit after Select All
  // (role: 'editMenu' expands to system defaults; custom items can't be inlined)
  const editMenu = menu.items.find((item) => item.label === 'Edit');
  if (editMenu?.submenu) {
    const selectAllIndex = editMenu.submenu.items.findIndex((item) => item.label === 'Select All');
    if (selectAllIndex >= 0) {
      editMenu.submenu.insert(
        selectAllIndex + 1,
        new MenuItem({ label: 'Find', submenu: Menu.buildFromTemplate(findSubmenu) })
      );
    }
  }

  // Add Always on Top to Window menu.
  const windowMenu = menu.items.find((item) => item.label === 'Window');
  if (windowMenu?.submenu) {
    const alwaysOnTopItem = new MenuItem({
      label: 'Always on Top',
      type: 'checkbox',
      // NO accelerator. This used to be Cmd+Shift+T — which is the universal
      // browser "reopen the last closed tab" chord, one keystroke away from this
      // app's own Cmd+T ("New Chat"). Users hit it by reflex and silently pinned
      // whatever window was focused (at launch, the first one) above every other
      // window — their own apps AND their other BioRouter windows — with no way
      // to tell why clicking elsewhere no longer brought a window forward. The
      // feature stays reachable from this menu; it no longer has a footgun chord.
      click(menuItem) {
        const win = BrowserWindow.getFocusedWindow();
        if (!win) {
          // Nothing focused: keep the checkmark honest rather than leaving it
          // reflecting a window that is no longer there.
          menuItem.checked = false;
          return;
        }
        // Decide from THIS window's real level, not from the shared menu item's
        // `checked`. The item is one global object across every window, so a
        // stale `checked` (set while a DIFFERENT window was focused) would
        // otherwise pin/un-pin the wrong window — the exact reason a pinned
        // window could not be released from any other window.
        const next = !win.isAlwaysOnTop();
        if (isMac) win.setAlwaysOnTop(next, 'floating');
        else win.setAlwaysOnTop(next);
        menuItem.checked = next;
      },
    });
    windowMenu.submenu.append(alwaysOnTopItem);

    // Keep the checkmark tracking whichever window is now focused, so the toggle
    // always acts on — and always reflects — the window the user is looking at.
    app.on('browser-window-focus', (_event, win) => {
      alwaysOnTopItem.checked = win.isAlwaysOnTop();
    });
  }

  Menu.setApplicationMenu(menu);
}

/**
 * Re-claim `biorouter://` if something else holds it.
 *
 * Older dev builds registered the bare Electron shell as the handler, which
 * silently broke every shared workflow link on that machine — the shell has no
 * app to run, so it launches and exits. Re-asserting on each packaged launch
 * heals those machines without the user having to know any of this.
 */
function ensureDeepLinkHandler() {
  if (!app.isPackaged) return;
  try {
    if (app.isDefaultProtocolClient('biorouter')) return;
    const reclaimed = app.setAsDefaultProtocolClient('biorouter');
    log.info(
      `[Main] biorouter:// was claimed by another app; reclaim ${reclaimed ? 'ok' : 'failed'}`
    );
  } catch (error) {
    log.warn('[Main] Could not verify biorouter:// handler registration:', error);
  }
}

async function appMain() {
  await configureProxy();

  ensureDeepLinkHandler();

  // Ensure Windows shims are available before any MCP processes are spawned
  await ensureWinShims();

  registerUpdateIpcHandlers();
  registerDependencyIpcHandlers();
  registerCliInstallHandlers();

  const appEntryUrl = rendererEntryUrl();
  session.defaultSession.setPermissionCheckHandler(
    (_webContents, permission, requestingOrigin, details) =>
      isAllowedRendererPermission(
        permission,
        details.requestingUrl || requestingOrigin,
        appEntryUrl,
        [details.mediaType ?? 'unknown']
      )
  );
  session.defaultSession.setPermissionRequestHandler(
    (_webContents, permission, callback, details) => {
      const mediaTypes = 'mediaTypes' in details ? (details.mediaTypes ?? []) : [];
      callback(
        isAllowedRendererPermission(permission, details.requestingUrl, appEntryUrl, mediaTypes)
      );
    }
  );

  const buildConnectSrc = (): string => {
    const sources = [
      "'self'",
      'http://127.0.0.1:*',
      // BR-71 §4.3: the workspace channel (`hooks/useWorkspaceChannel.ts`) is
      // the renderer's only WebSocket to the daemon. CSP will not stretch an
      // `http:` source over a `ws:` URL — CSP3 §6.6.2.6 relaxes `http`→`https`
      // and `ws`→`wss`/`http`/`https`, never `http`→`ws` — so without this the
      // socket is blocked before it leaves the renderer and every
      // `workspace_list` reports `gui_attached: false` with the GUI on screen.
      'ws://127.0.0.1:*',
      'https://api.github.com',
      'https://github.com',
      'https://objects.githubusercontent.com',
    ];

    const settings = loadSettings();
    if (settings.externalBiorouterd?.enabled && settings.externalBiorouterd.url) {
      try {
        const externalUrl = new URL(settings.externalBiorouterd.url);
        sources.push(externalUrl.origin);
        // Same reason as the loopback ws entry above: an external backend needs
        // the ws form of its own origin, derived exactly the way the hook
        // derives the socket URL (`getApiUrl(...).replace(/^http/, 'ws')`).
        sources.push(externalUrl.origin.replace(/^http/, 'ws'));
      } catch {
        console.warn('Invalid external biorouterd URL in settings, skipping CSP entry');
      }
    }

    return sources.join(' ');
  };

  // Add CSP headers to all sessions
  session.defaultSession.webRequest.onHeadersReceived((details, callback) => {
    // Standalone artifact files contain a sandboxed srcdoc preview. Its inline
    // chart runtime must execute, but the artifact must not fetch remote code,
    // beacon data, or connect to local services.
    const isArtifactWindow =
      details.url.startsWith('file://') && details.url.includes('biorouter-artifacts');

    const csp = isArtifactWindow
      ? ARTIFACT_WRAPPER_CSP
      : "default-src 'self';" +
        "style-src 'self' 'unsafe-inline';" +
        // `wasm-unsafe-eval`, NOT `unsafe-eval`. pdf.js 6 ships its JPEG 2000,
        // JBIG2 and colour-management decoders as WebAssembly, and Chromium
        // blocks WASM outright unless this token is present. It does NOT permit
        // `eval` or `new Function`; pdf.js dropped its need for those in
        // 5.7.284, so the older advice to add `unsafe-eval` is stale and would
        // widen this policy for nothing.
        "script-src 'self' 'wasm-unsafe-eval';" +
        "img-src 'self' data: blob: https:;" +
        `connect-src ${buildConnectSrc()};` +
        "object-src 'none';" +
        "frame-src 'self' blob: https: http:;" +
        "font-src 'self' data: https:;" +
        "media-src 'self' mediastream:;" +
        "form-action 'none';" +
        "base-uri 'self';" +
        "manifest-src 'self';" +
        "worker-src 'self';" +
        'upgrade-insecure-requests;';

    callback({
      responseHeaders: {
        ...details.responseHeaders,
        'Content-Security-Policy': csp,
      },
    });
  });

  try {
    globalShortcut.register('CommandOrControl+Alt+Shift+G', () => {
      createLauncher();
    });
  } catch (e) {
    console.error('Error registering launcher hotkey:', e);
  }

  try {
    globalShortcut.register('CommandOrControl+Alt+G', () => {
      focusWindow();
    });
  } catch (e) {
    console.error('Error registering focus window hotkey:', e);
  }

  session.defaultSession.webRequest.onBeforeSendHeaders((details, callback) => {
    details.requestHeaders['Origin'] = 'http://localhost:5173';
    callback({ cancel: false, requestHeaders: details.requestHeaders });
  });

  // Create tray if enabled in settings
  const settings = loadSettings();
  if (settings.showMenuBarIcon) {
    createTray();
  }

  // Handle dock icon visibility (macOS only)
  if (process.platform === 'darwin' && !settings.showDockIcon && settings.showMenuBarIcon) {
    app.dock?.hide();
  }

  const { dirPath } = parseArgs();

  if (!openUrlHandledLaunch) {
    await createNewWindow(app, dirPath);
  } else {
    log.info('[Main] Skipping window creation in appMain - open-url already handled launch');
  }

  // Watch for the class of bug this whole area was fixed for (#88): anything
  // that blocks the main thread now says so in the log instead of being visible
  // only as "the app feels stuck".
  startMainThreadWatchdog();

  // Background startup work — deliberately staggered.
  //
  // These three subsystems all used to fire into the same few seconds while the
  // renderer was still doing its first meaningful paint (#88). Their probes are
  // no longer synchronous, so they cannot freeze the main thread any more, but
  // they still compete for CPU, disk and network with the window the user is
  // trying to use. The gaps below keep them out of each other's way, and out of
  // the renderer's way, in a fixed order:
  //
  //   T+2s   auto-updater setup      (its own first network check lands at T+7s)
  //   T+6s   dependency check        (spawns `biorouter doctor`, ~3.5 s of work)
  //   T+15s  extension update check  (GitHub API per extension, then `uv sync`)
  //
  // Raising a delay is safe. Lowering one puts work back into the paint window.
  setTimeout(() => {
    if (shouldSetupUpdater()) {
      log.info('Setting up auto-updater after window creation...');
      try {
        setupAutoUpdater();
      } catch (error) {
        log.error('Error setting up auto-updater:', error);
      }
    }
  }, STARTUP_UPDATER_SETUP_DELAY_MS);

  setupDependencyChecker(STARTUP_DEPENDENCY_CHECK_DELAY_MS);

  scheduleExtensionUpdateCheck(STARTUP_EXTENSION_CHECK_DELAY_MS);

  // Setup macOS dock menu
  if (process.platform === 'darwin') {
    const dockMenu = Menu.buildFromTemplate([
      {
        label: 'New Window',
        click: () => {
          createNewWindow(app);
        },
      },
    ]);
    app.dock?.setMenu(dockMenu);
  }

  buildApplicationMenu();

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createNewWindow(app);
    }
  });

  ipcMain.on(
    'create-chat-window',
    async (event, query, dir, version, resumeSessionId, viewType, workflowId) => {
      if (!dir?.trim()) {
        const recentDirs = loadRecentDirs();
        dir = recentDirs.length > 0 ? recentDirs[0] : undefined;
      }

      // Offset the new window from the one that triggered it (e.g. the Branch
      // button) so it's clearly a distinct second window, then bring it to the
      // front. The originating window stays exactly where it is.
      //
      // ⚠ **Anchor on the SENDER, not on whatever happens to be focused**
      // (#78). The handler discarded its `event` and asked
      // `BrowserWindow.getFocusedWindow()`, which is a different question: with
      // several windows open, an agent-driven `placement: "window"` arrives
      // without the user having clicked anything, so the focused window is
      // whichever one they last touched rather than the one that asked. The new
      // window then appeared offset from a stranger. `event.sender` names the
      // renderer that actually sent this, which is the only honest anchor.
      // `event?.sender`, because a caller that reaches this listener without an
      // Electron IPC event is not hypothetical: the File menu did exactly that
      // and took Cmd+N down with it. The sender is still the only honest
      // anchor when there IS one (#78); this just makes its absence fall
      // through to the next candidate instead of throwing.
      const anchor =
        (event?.sender ? BrowserWindow.fromWebContents(event.sender) : null) ??
        BrowserWindow.getFocusedWindow() ??
        BrowserWindow.getAllWindows()[0];
      const win = await createChat(
        app,
        query,
        dir,
        version,
        resumeSessionId,
        viewType,
        undefined,
        undefined,
        workflowId
      );
      if (win) {
        if (anchor && anchor !== win && !anchor.isDestroyed()) {
          const b = anchor.getBounds();
          win.setBounds({ x: b.x + 40, y: b.y + 40, width: b.width, height: b.height });
        }
        win.show();
        win.focus();
        win.moveTop();
      }
    }
  );

  ipcMain.on(
    'create-diverged-chat-window',
    async (event, dir, resumeSessionId, resumeSessionTitle) => {
      if (!resumeSessionId) {
        log.error('[Main] create-diverged-chat-window missing session id');
        return;
      }
      if (!dir?.trim()) {
        const recentDirs = loadRecentDirs();
        dir = recentDirs.length > 0 ? recentDirs[0] : undefined;
      }
      const senderWindow = BrowserWindow.fromWebContents(event.sender);
      await openDivergedChatWindow(resumeSessionId, dir, senderWindow, resumeSessionTitle);
    }
  );

  // ── Tab tear-off and merge ────────────────────────────────────────────────
  // Four channels in, two out. See the block above `createLauncher` for the
  // registry and the ownership rules; these are only the doors.

  ipcMain.on('tab-drag:register-bands', (event, bands) => {
    const win = BrowserWindow.fromWebContents(event.sender);
    // ONLY CHAT WINDOWS REGISTER. A launcher, artifact or app window has no tab
    // strip and must never become a merge target — `windowMap` is exactly the
    // set `createChat` builds, so membership is the test. (The converse is a
    // known limitation stated in D3: one of OUR non-chat windows sitting above a
    // chat window's strip cannot occlude it, because it is not here to be seen.)
    if (!win || win.isDestroyed() || !windowMap.has(win.id)) return;
    stripBandRegistry.register(win.id, {
      contentBounds: win.getContentBounds(),
      bands: sanitizeStripBands(bands),
    });
  });

  // EVERY IPC PAYLOAD BELOW IS TYPED `unknown` ON PURPOSE. `ipcMain`'s own
  // signature says `any`, and that is exactly how a `{screenX, screenY}` wire
  // point was handed to a function expecting `{x, y}` with `tsc` staying clean
  // and every drop silently resolving to `detach`. `unknown` makes the
  // converters in `windowDrag.ts` the only way in.
  ipcMain.on('tab-drag:move', (event, point: unknown) => {
    const source = BrowserWindow.fromWebContents(event.sender);
    if (!source || source.isDestroyed()) return;
    const rawPoint = screenPointFromWire(point);
    if (!rawPoint) return;
    // The only message that says who is holding the pointer, so it is the only
    // place the broker can learn the drag SOURCE — which is what lets a source
    // that dies mid-drag take its caret with it (D4).
    tabDragBroker.noteDragSource(source.id);
    refreshRegisteredContentBounds();
    const phase = resolveDropTargetForRawPoint(
      rawPoint,
      tabDragGeometry(),
      stripBandRegistry,
      source.id
    );
    if (phase.kind === 'merge') {
      tabDragBroker.showPreview(phase.targetWindowId, screenPointToWire(rawPoint));
    } else {
      tabDragBroker.clearPreview();
    }

    // ── The ghost that leaves the window (issue #75) ───────────────────────
    // NORMALISED AGAIN RATHER THAN THREADED THROUGH THE HIT TEST. `resolveDrop-
    // TargetForRawPoint` does its own `normalizeToDip` internally, and unpicking
    // it to share one conversion would restructure the proven path for a pure
    // function that is the identity on macOS and one native call elsewhere. What
    // must NOT happen is positioning a window from `rawPoint`: `BrowserWindow`
    // bounds are DIP and raw screen coordinates are not under Windows
    // per-monitor DPI (windowDrag.ts D4).
    //
    // `detach` only. A `merge` already has a caret in the target window saying
    // where the tab will land, and a ghost flying over it would be a second,
    // competing answer to the same question; `local` is the in-window drag,
    // which the DOM ghost owns.
    if (phase.kind === 'detach') {
      dragGhostWindows.follow(source.id, normalizeToDip(rawPoint, tabDragGeometry()));
    } else {
      dragGhostWindows.release(`phase ${phase.kind}`);
    }
  });

  ipcMain.on('tab-drag:end', () => {
    tabDragBroker.endDrag();
    // Also the message the source sends when the cursor comes BACK INSIDE
    // (`ChatGroupsShell` reports a `local` phase through this same door), so
    // this is the ordinary "the ghost is no longer wanted" path, not only the
    // teardown one.
    dragGhostWindows.release('drag ended');
  });

  // ATTRIBUTED TO THE SENDER. `tabDragAckMerge` is on every renderer's preload,
  // so a bare request id from anyone would do: the broker believes an ack only
  // from the webContents the request was actually sent to.
  ipcMain.on('tab-drag:merge-ack', (event, requestId: unknown, inserted: unknown) => {
    if (typeof requestId !== 'number') return;
    const accepted = tabDragBroker.ackMerge(requestId, !!inserted, event.sender.id);
    if (!accepted) {
      log.warn(
        `[tab-drag] ignored merge ack ${requestId} from webContents ${event.sender.id}` +
          ' (unknown, already settled, or not the window it was sent to)'
      );
    }
  });

  ipcMain.handle('tab-drag:commit', async (event, request: unknown) => {
    // FIRST STATEMENT, ahead of every early return and of the merge round trip:
    // the button is up, so the ghost has nothing left to represent whatever this
    // handler decides. Waiting for the outcome would leave it hanging over the
    // desktop for the whole 2s ack window of a merge that may yet be refused,
    // and returning early would leave it there for good.
    dragGhostWindows.release('drag committed');

    const source = BrowserWindow.fromWebContents(event.sender);
    // Every early return is `noop`, which means "keep the tab". There is no
    // failure here that should cost the user a chat.
    if (!source || source.isDestroyed()) return { outcome: 'noop' };
    const req = (request ?? {}) as TabDragCommitRequestWire;
    const rawPoint = screenPointFromWire(req.point);
    if (!rawPoint) return { outcome: 'noop' };
    const wirePoint = screenPointToWire(rawPoint);

    // The caret goes the moment the button is released, whatever happens next.
    tabDragBroker.endDrag();
    refreshRegisteredContentBounds();
    const phase = resolveDropTargetForRawPoint(
      rawPoint,
      tabDragGeometry(),
      stripBandRegistry,
      source.id
    );
    if (phase.kind === 'local') return { outcome: 'noop' };

    if (phase.kind === 'merge') {
      const inserted = await tabDragBroker.requestMerge(phase.targetWindowId, req.tab, wirePoint);
      if (!inserted) return { outcome: 'noop' };
      // NOW it may be raised — the gesture is over, so there is no capture left
      // to steal (D3's "not raised, not focused" applies only during preview).
      // Re-read the window: the ack round trip is the one place in this handler
      // where the target can have gone away since it was resolved.
      const target = windowMap.get(phase.targetWindowId);
      if (target && !target.isDestroyed()) {
        target.show();
        target.focus();
        target.moveTop();
      }
      return { outcome: 'merge' };
    }

    // Which releases may NOT become a new window — D5's lone tab, and a tab
    // with no session behind it. The rule itself lives in `windowDrag.ts` with
    // the reasoning and the tests; both answers are `noop`, which the renderer
    // already reads as "keep the tab". Neither applies to the merge branch
    // above: moving either tab INTO another window is exactly the gesture.
    const refusal = detachRefusal(req);
    if (refusal) {
      log.info(`[tab-drag] tear-off refused (${refusal}); the tab stays where it is`);
      return { outcome: 'noop' };
    }

    const bounds = tornOffWindowBoundsForRawPoint(
      rawPoint,
      grabOffsetFromWire(req.grabOffset),
      source.getBounds(),
      tabDragGeometry()
    );
    // Seeded through the proven path — the same one Diverge and "Open in new
    // window" already use daily. `show: false` then show/focus/moveTop mirrors
    // openDivergedChatWindow so the window does not flash at its default
    // position before moving to the drop point.
    const win = await createChat(
      app,
      undefined,
      req.tab?.cwd,
      undefined,
      req.tab?.sessionId,
      'pair',
      undefined,
      undefined,
      req.tab?.workflowId,
      undefined,
      {
        initialBounds: bounds,
        show: false,
        manageWindowState: false,
        ...(req.tab?.title ? { resumeSessionTitle: req.tab.title } : {}),
      }
    );
    if (!win) return { outcome: 'noop' };
    win.show();
    win.focus();
    win.moveTop();
    return { outcome: 'detach' };
  });

  // THE ONE OWNER OF `close-window`, and it closes the SENDER — never
  // `getFocusedWindow()`. The two are the same window almost always, which is
  // why a second handler in utils/workflowHash.ts that closed the focused
  // window sat here undetected: the only caller where they DIFFER is the tab
  // merge above, which focuses the target immediately before the source asks
  // to close itself. Both handlers ran, each closed a different window, and a
  // merge drop took down the target as well as the source — down to zero
  // windows when those were the last two.
  ipcMain.on('close-window', (event) => {
    const window = BrowserWindow.fromWebContents(event.sender);
    if (window && !window.isDestroyed()) {
      window.close();
    }
  });

  // `ipcMain.on` is ADDITIVE: a second module registering this channel does not
  // replace this handler, it runs alongside it, and nothing anywhere reports
  // that. Assert single ownership at startup so the next duplicate is loud on
  // the first launch instead of surfacing years later as windows vanishing.
  // Window-lifecycle channels are the ones where a second opinion is
  // destructive, so they are the ones guarded.
  for (const soleOwnerChannel of ['close-window'] as const) {
    const owners = ipcMain.listenerCount(soleOwnerChannel);
    if (owners !== 1) {
      log.error(
        `[ipc] '${soleOwnerChannel}' has ${owners} listeners, expected exactly 1. ` +
          'A duplicate handler will act on a window this one did not mean. ' +
          'See utils/workflowHash.ts for the original offender.'
      );
    }
  }

  ipcMain.on('notify', (event, data) => {
    try {
      // Validate notification data
      if (!data || typeof data !== 'object') {
        console.error('Invalid notification data');
        return;
      }

      // Validate title and body
      if (typeof data.title !== 'string' || typeof data.body !== 'string') {
        console.error('Invalid notification title or body');
        return;
      }

      // Limit the length of title and body
      const MAX_LENGTH = 1000;
      if (data.title.length > MAX_LENGTH || data.body.length > MAX_LENGTH) {
        console.error('Notification title or body too long');
        return;
      }

      // Remove any HTML tags for security
      const sanitizeText = (text: string) => text.replace(/<[^>]*>/g, '');

      console.log('NOTIFY', data);
      const notification = new Notification({
        title: sanitizeText(data.title),
        body: sanitizeText(data.body),
      });

      // Add click handler to focus the window
      notification.on('click', () => {
        const window = BrowserWindow.fromWebContents(event.sender);
        if (window) {
          if (window.isMinimized()) {
            window.restore();
          }
          window.show();
          window.focus();
        }
      });

      notification.show();
    } catch (error) {
      console.error('Error showing notification:', error);
    }
  });

  ipcMain.on('logInfo', (_event, info) => {
    try {
      // Validate log info
      if (info === undefined || info === null) {
        console.error('Invalid log info: undefined or null');
        return;
      }

      // Convert to string if not already
      const logMessage = String(info);

      // Limit log message length
      const MAX_LENGTH = 10000; // 10KB limit
      if (logMessage.length > MAX_LENGTH) {
        console.error('Log message too long');
        return;
      }

      // Log the sanitized message
      log.info('from renderer:', logMessage);
    } catch (error) {
      console.error('Error logging info:', error);
    }
  });

  ipcMain.on('broadcast-theme-change', (event, themeData) => {
    const senderWindow = BrowserWindow.fromWebContents(event.sender);
    const allWindows = BrowserWindow.getAllWindows();

    allWindows.forEach((window) => {
      if (window.id !== senderWindow?.id) {
        window.webContents.send('theme-changed', themeData);
      }
    });
  });

  ipcMain.on('reload-app', (event) => {
    // Get the window that sent the event
    const window = BrowserWindow.fromWebContents(event.sender);
    if (window) {
      window.reload();
    }
  });

  // Handle metadata fetching from main process
  ipcMain.handle('fetch-metadata', async (_event, url) => {
    try {
      // Each hop is validated: a public URL that 302s to 127.0.0.1 or
      // 169.254.169.254 would otherwise turn this handler into an SSRF proxy.
      let target = await assertPublicHttpUrl(url);
      let response: Response | undefined;
      for (let hop = 0; hop < 5; hop++) {
        response = await fetch(target.href, {
          redirect: 'manual',
          headers: {
            'User-Agent': 'Mozilla/5.0 (compatible; Biorouter/1.0)',
          },
        });
        const location = response.headers.get('location');
        if (response.status >= 300 && response.status < 400 && location) {
          target = await assertPublicHttpUrl(new URL(location, target).href);
          continue;
        }
        break;
      }
      if (!response) throw new Error('Too many redirects');

      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }

      // Set a reasonable size limit (e.g., 10MB)
      const MAX_SIZE = 10 * 1024 * 1024; // 10MB
      const contentLength = parseInt(response.headers.get('content-length') || '0');
      if (contentLength > MAX_SIZE) {
        throw new Error('Response too large');
      }

      const text = await response.text();
      if (text.length > MAX_SIZE) {
        throw new Error('Response too large');
      }

      return text;
    } catch (error) {
      console.error('Error fetching metadata:', error);
      throw error;
    }
  });

  ipcMain.on('open-in-chrome', async (event, url) => {
    try {
      // Despite the legacy channel name, use Electron's non-shell URL opener.
      // Passing a renderer URL through cmd.exe made &, | and ^ executable.
      const window = BrowserWindow.fromWebContents(event.sender);
      if (window) await openExternalBrowserNavigation(window, url);
    } catch (error) {
      console.error('Error opening URL in browser:', error);
    }
  });

  // Handle app restart
  ipcMain.on('restart-app', () => {
    app.relaunch();
    app.exit(0);
  });

  // Handler for getting app version
  ipcMain.on('get-app-version', (event) => {
    event.returnValue = app.getVersion();
  });

  ipcMain.handle('open-directory-in-explorer', async (event, dirPath: string) => {
    try {
      if (typeof dirPath !== 'string' || dirPath.trim() === '') return false;
      const expanded = path.resolve(expandBiorouterPath(dirPath));

      // The same containment `read-artifact-file` takes, and for the same
      // reason. Without it this handler was strictly MORE permissive than the
      // preview reader beside it: the panel refused to *show* a file it would
      // happily hand to the OS — same panel, same file, two answers. The path
      // is named by the agent, so it is not trusted input.
      if (!isAllowedFilePath(expanded, workingDirForSender(event))) {
        console.error(
          `open-directory-in-explorer blocked: '${expanded}' is outside allowed directories`
        );
        return false;
      }

      // A plain directory opens a file manager and that is the end of it. Any
      // other target is handed to whatever the OS has registered for it, and
      // that is a different act: a generated `.html` opens in the default
      // browser as `file://` with no CSP and no sandbox, and a `.command`,
      // `.exe`, `.desktop` or `.app` bundle is *executed*. Those get the same
      // treatment `open-external` gives a URL — a native dialog naming the
      // target, defaulting to Cancel — because containment alone does not
      // distinguish "show me this folder" from "run this".
      //
      // The extension test is what keeps macOS package bundles out of the
      // no-dialog path: `/Applications/Anything.app` is a directory.
      if (!(await isPlainDirectory(expanded))) {
        const confirmed = await confirmSystemHandlerOpen(event, expanded);
        if (!confirmed) return false;
      }

      const err = await shell.openPath(expanded);
      // shell.openPath returns an empty string on success, error message on failure
      if (err) console.error('Error opening directory in explorer:', err);
      return !err;
    } catch (error) {
      console.error('Error opening directory in explorer:', error);
      return false;
    }
  });

  // Standalone previews are offline. Inline the small, fixed set of libraries
  // emitted by Auto Visualiser before applying its network-denying CSP. The
  // asset list, the tag patterns and the substitution itself live in
  // `utils/artifactCdnAssets.ts` so the Rust side can assert against them.
  const artifactCdnAssetCache = new Map<string, Promise<string>>();

  const fetchArtifactCdnAsset = (url: string): Promise<string> => {
    let cached = artifactCdnAssetCache.get(url);
    if (!cached) {
      cached = fetch(url).then((response) => {
        if (!response.ok) {
          throw new Error(`HTTP ${response.status}`);
        }
        return response.text();
      });
      artifactCdnAssetCache.set(url, cached);
    }
    return cached;
  };

  const inlineKnownArtifactCdnAssets = (rawHtml: string): Promise<string> =>
    inlineArtifactCdnAssets(rawHtml, fetchArtifactCdnAsset, (url, error) => {
      console.warn(`Could not inline artifact CDN asset ${url}:`, error);
    });

  type OpenArtifactPayload = {
    html: string;
    title?: string;
    width?: number;
    height?: number;
    theme?: 'light' | 'dark';
  };

  const normalizeArtifactPayload = (payload: unknown): OpenArtifactPayload | null => {
    if (!payload || typeof payload !== 'object') return null;
    const value = payload as Record<string, unknown>;
    if (
      typeof value.html !== 'string' ||
      Buffer.byteLength(value.html, 'utf8') > 16 * 1024 * 1024
    ) {
      return null;
    }
    const title = typeof value.title === 'string' ? sanitizeUntrustedLabel(value.title) : undefined;
    const finiteDimension = (dimension: unknown) =>
      typeof dimension === 'number' && Number.isFinite(dimension) ? dimension : undefined;
    return {
      html: value.html,
      title,
      width: finiteDimension(value.width),
      height: finiteDimension(value.height),
      theme: value.theme === 'dark' ? 'dark' : value.theme === 'light' ? 'light' : undefined,
    };
  };

  const prepareArtifactHtml = async (rawHtml: string): Promise<string> => {
    return inlineKnownArtifactCdnAssets(rawHtml);
  };

  ipcMain.handle('prepare-artifact-html', async (_event, payload: unknown) => {
    const normalized = normalizeArtifactPayload(payload);
    if (!normalized) throw new Error('Invalid or oversized artifact preview');
    return { html: await prepareArtifactHtml(normalized.html) };
  });

  let artifactTempDirectoryPromise: Promise<string> | null = null;
  const artifactTempDirectory = (): Promise<string> => {
    artifactTempDirectoryPromise ??= fs
      .mkdtemp(path.join(os.tmpdir(), 'biorouter-artifacts-'))
      .then(async (directory) => {
        await fs.chmod(directory, 0o700);
        app.once('before-quit', () => {
          void fs.rm(directory, { recursive: true, force: true });
        });
        return directory;
      });
    return artifactTempDirectoryPromise;
  };

  const writeArtifactTempFile = async (html: string): Promise<string> => {
    const artifactDir = await artifactTempDirectory();
    const artifactFile = path.join(artifactDir, `artifact-${crypto.randomUUID()}.html`);
    await fs.writeFile(artifactFile, html, { encoding: 'utf-8', mode: 0o600, flag: 'wx' });
    return artifactFile;
  };

  const openArtifactInBrowser = async (payload: OpenArtifactPayload) => {
    try {
      const html = wrapArtifactForBrowser(
        injectArtifactHostTheme(
          await prepareArtifactHtml(payload.html),
          payload.theme === 'dark' ? 'dark' : 'light'
        )
      );
      const artifactFile = await writeArtifactTempFile(html);
      await shell.openExternal(pathToFileURL(artifactFile).href);
      return { ok: true };
    } catch (error) {
      console.error('Error opening artifact in browser:', error);
      return { ok: false };
    }
  };

  ipcMain.handle('open-artifact-in-browser', (_event, payload: unknown) => {
    const normalized = normalizeArtifactPayload(payload);
    return normalized ? openArtifactInBrowser(normalized) : { ok: false };
  });
}

app.whenReady().then(async () => {
  try {
    if (process.platform === 'darwin') {
      const dockIconPath = resolveImagePath('icon.png');
      if (dockIconPath) app.dock?.setIcon(dockIconPath);
    }
    await appMain();
  } catch (error) {
    // Log BEFORE the dialog. `showErrorBox` is modal and blocks the main thread
    // until someone dismisses it, so on a headless or automated launch the only
    // record of a fatal startup error was a box nobody could see and no log line
    // at all — the failure looked like a silent hang.
    log.error('[Main] Fatal error during startup:', error);
    if (error instanceof Error && error.stack) log.error(error.stack);
    dialog.showErrorBox('Biorouter Error', `Failed to create main window: ${error}`);
    app.quit();
  }
});

async function getAllowList(): Promise<string[]> {
  if (!process.env.BIOROUTER_ALLOWLIST) {
    return [];
  }

  const response = await fetch(process.env.BIOROUTER_ALLOWLIST);

  if (!response.ok) {
    throw new Error(
      `Failed to fetch allowed extensions: ${response.status} ${response.statusText}`
    );
  }

  // Parse the YAML content
  const yamlContent = await response.text();
  const parsedYaml = yaml.parse(yamlContent);

  // Extract the commands from the extensions array
  if (parsedYaml && parsedYaml.extensions && Array.isArray(parsedYaml.extensions)) {
    const commands = parsedYaml.extensions.map(
      (ext: { id: string; command: string }) => ext.command
    );
    console.log(`Fetched ${commands.length} allowed extension commands`);
    return commands;
  } else {
    console.error('Invalid YAML structure:', parsedYaml);
    return [];
  }
}

app.on('will-quit', async () => {
  for (const [windowId, blockerId] of windowPowerSaveBlockers.entries()) {
    try {
      powerSaveBlocker.stop(blockerId);
      console.log(
        `[Main] Stopped power save blocker ${blockerId} for window ${windowId} during app quit`
      );
    } catch (error) {
      console.error(
        `[Main] Failed to stop power save blocker ${blockerId} for window ${windowId}:`,
        error
      );
    }
  }
  windowPowerSaveBlockers.clear();

  // Unregister all shortcuts when quitting
  globalShortcut.unregisterAll();

  try {
    await fs.access(biorouterTempDir); // Check if directory exists to avoid error on fs.rm if it doesn't

    // First, check for any symlinks in the directory and refuse to delete them
    let hasSymlinks = false;
    try {
      const files = await fs.readdir(biorouterTempDir);
      for (const file of files) {
        const filePath = path.join(biorouterTempDir, file);
        const stats = await fs.lstat(filePath);
        if (stats.isSymbolicLink()) {
          console.warn(`[Main] Found symlink in temp directory: ${filePath}. Skipping deletion.`);
          hasSymlinks = true;
          // Delete the individual file but leave the symlink
          continue;
        }

        // Delete regular files individually
        if (stats.isFile()) {
          await fs.unlink(filePath);
        }
      }

      // If no symlinks were found, it's safe to remove the directory
      if (!hasSymlinks) {
        await fs.rm(biorouterTempDir, { recursive: true, force: true });
        console.log('[Main] Pasted images temp directory cleaned up successfully.');
      } else {
        console.log(
          '[Main] Cleaned up files in temp directory but left directory intact due to symlinks.'
        );
      }
    } catch (err) {
      console.error('[Main] Error while cleaning up temp directory contents:', err);
    }
  } catch (error) {
    if (error && typeof error === 'object' && 'code' in error && error.code === 'ENOENT') {
      console.log('[Main] Temp directory did not exist during "will-quit", no cleanup needed.');
    } else {
      console.error(
        '[Main] Failed to clean up pasted images temp directory during "will-quit":',
        error
      );
    }
  }
});

app.on('window-all-closed', () => {
  // Only quit if we're not on macOS or don't have a tray icon
  if (process.platform !== 'darwin' || !tray) {
    app.quit();
  }
});
