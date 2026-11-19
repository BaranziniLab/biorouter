/**
 * extensionUpdater.ts
 *
 * On startup, scans installed extensions in the Biorouter config directory's
 * `extensions/` folder, reads each manifest.json for version + repository,
 * queries the GitHub releases API for the latest .brxt asset, and auto-updates
 * outdated ones in the background.
 *
 * Skips silently on network errors so offline users are unaffected.
 */

import * as path from 'path';
import * as os from 'os';
import * as fs from 'fs/promises';
import { BrowserWindow } from 'electron';
import { compareVersions } from 'compare-versions';
import AdmZip from 'adm-zip';
import { safeExtractZip } from './safeZip';
import log from './logger';
import { runProbe } from './dependencyChecker';
import { biorouterExtensionsDir as sharedExtensionsDir } from './biorouterPaths';

// ─── Types ────────────────────────────────────────────────────────────────────

interface ExtensionManifest {
  name: string;
  display_name: string;
  description: string;
  version: string;
  entry_point: string;
  repository: string; // e.g. "https://github.com/owner/repo"
}

interface GitHubRelease {
  tag_name: string;
  assets: Array<{
    name: string;
    browser_download_url: string;
    size: number;
  }>;
}

interface ExtensionUpdateInfo {
  name: string;
  displayName: string;
  currentVersion: string;
  latestVersion: string;
  downloadUrl: string;
  installDir: string;
}

export interface ExtensionUpdateEvent {
  type:
    | 'update-found'
    | 'update-start'
    | 'update-progress'
    | 'update-done'
    | 'update-error'
    | 'all-done';
  ext?: string;
  displayName?: string;
  from?: string;
  to?: string;
  percent?: number;
  error?: string;
  updatedCount?: number;
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/**
 * The `extensions/` directory this updater reads, writes and runs `uv sync` in.
 *
 * ⚠ **Resolved by `utils/biorouterPaths.ts`, per call, and never at module
 * load.** This used to be a module-level constant built from `os.homedir()`,
 * which made the updater ignore `BIOROUTER_PATH_ROOT`. That is not a cosmetic
 * inconsistency: this module is a *writer* — it extracts a downloaded `.brxt`
 * over `installDir` and then runs `uv sync` there — so a sandboxed dev build
 * pointed at a scratch root still enumerated, and would have overwritten, the
 * developer's real extensions 15 s after every launch. Measured live: a
 * sandboxed build listed all nine of the developer's installed extensions.
 *
 * ⚠ This note used to call the updater "the one main-process module that
 * ignored `BIOROUTER_PATH_ROOT`". It was not: `main.ts`'s two `.brxt` handlers
 * (one of which recursively deletes) and `extensionProvenance.ts` all derived
 * `~/.config/biorouter` themselves, and the sentence was exactly what a
 * reviewer would use to conclude #146 was closed. It is closed now because
 * every one of them resolves through the shared module — the fix was to remove
 * the other derivations, not to describe them away.
 */
export function biorouterExtensionsDir(): string {
  return sharedExtensionsDir();
}

const GITHUB_API_BASE = 'https://api.github.com/repos';

/** Parse owner/repo from a GitHub URL. Returns null for non-GitHub URLs. */
function parseGitHubRepo(repoUrl: string): { owner: string; repo: string } | null {
  try {
    const url = new URL(repoUrl);
    if (!url.hostname.includes('github.com')) return null;
    const parts = url.pathname.replace(/^\//, '').replace(/\/$/, '').split('/');
    if (parts.length < 2) return null;
    const [owner, repo] = parts;
    if (!owner || !repo) return null;
    return { owner, repo: repo.replace(/\.git$/, '') };
  } catch {
    return null;
  }
}

/** Clean version string: strip leading 'v' and whitespace. */
function cleanVersion(v: string): string {
  return v.trim().replace(/^v/i, '');
}

/** Fetch with a timeout and a User-Agent header (GitHub requires it). */
async function fetchJson<T>(url: string, timeoutMs = 15000): Promise<T> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const res = await fetch(url, {
      signal: controller.signal,
      headers: { 'User-Agent': 'Biorouter-ExtensionUpdater/1.0' },
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}: ${url}`);
    return (await res.json()) as T;
  } finally {
    clearTimeout(timer);
  }
}

/** Download a URL to a file, tracking progress. */
async function downloadFile(
  url: string,
  dest: string,
  onProgress: (percent: number) => void
): Promise<void> {
  const res = await fetch(url, {
    headers: { 'User-Agent': 'Biorouter-ExtensionUpdater/1.0' },
  });
  if (!res.ok) throw new Error(`Download failed: HTTP ${res.status}`);

  const total = Number(res.headers.get('content-length') || 0);
  let received = 0;
  const chunks: Uint8Array[] = [];

  const reader = res.body?.getReader();
  if (!reader) throw new Error('No response body');

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    received += value.length;
    if (total > 0) onProgress(Math.round((received / total) * 100));
  }

  const buf = Buffer.concat(chunks);
  await fs.writeFile(dest, buf);
}

// ─── Core update flow ─────────────────────────────────────────────────────────

async function scanInstalledExtensions(): Promise<
  Array<{ manifest: ExtensionManifest; installDir: string }>
> {
  const extensionsDir = biorouterExtensionsDir();
  try {
    const entries = await fs.readdir(extensionsDir, { withFileTypes: true });
    const results: Array<{ manifest: ExtensionManifest; installDir: string }> = [];
    for (const entry of entries) {
      if (!entry.isDirectory()) continue;
      const manifestPath = path.join(extensionsDir, entry.name, 'manifest.json');
      try {
        const raw = await fs.readFile(manifestPath, 'utf8');
        const manifest = JSON.parse(raw) as ExtensionManifest;
        if (manifest.name && manifest.version && manifest.repository) {
          results.push({ manifest, installDir: path.join(extensionsDir, entry.name) });
        }
      } catch {
        // skip malformed manifests
      }
    }
    return results;
  } catch {
    return [];
  }
}

async function checkExtensionUpdate(
  manifest: ExtensionManifest,
  installDir: string
): Promise<ExtensionUpdateInfo | null> {
  const parsed = parseGitHubRepo(manifest.repository);
  if (!parsed) return null;

  const { owner, repo } = parsed;
  const apiUrl = `${GITHUB_API_BASE}/${owner}/${repo}/releases/latest`;

  let release: GitHubRelease;
  try {
    release = await fetchJson<GitHubRelease>(apiUrl);
  } catch (err) {
    log.warn(
      `[ExtensionUpdater] Cannot fetch release for ${manifest.name}:`,
      (err as Error).message
    );
    return null;
  }

  const latestVersion = cleanVersion(release.tag_name);
  const currentVersion = cleanVersion(manifest.version);

  let cmp: number;
  try {
    cmp = compareVersions(latestVersion, currentVersion);
  } catch {
    cmp = 0;
  }
  if (cmp <= 0) return null; // already up-to-date

  // Find a .brxt asset
  const asset = release.assets.find((a) => a.name.endsWith('.brxt'));
  if (!asset) {
    log.info(`[ExtensionUpdater] No .brxt asset in release ${latestVersion} for ${manifest.name}`);
    return null;
  }

  return {
    name: manifest.name,
    displayName: manifest.display_name,
    currentVersion,
    latestVersion,
    downloadUrl: asset.browser_download_url,
    installDir,
  };
}

async function applyUpdate(
  info: ExtensionUpdateInfo,
  send: (e: ExtensionUpdateEvent) => void
): Promise<boolean> {
  let tmpDir: string | undefined;
  let backupRoot: string | undefined;
  let backupDir: string | undefined;
  let backupReady = false;
  let retainBackup = false;
  let error: string | undefined;

  try {
    tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), 'biorouter-ext-update-'));
    const tmpBrxt = path.join(tmpDir, 'extension.brxt');

    send({
      type: 'update-start',
      ext: info.name,
      displayName: info.displayName,
      from: info.currentVersion,
      to: info.latestVersion,
    });

    // Download
    await downloadFile(info.downloadUrl, tmpBrxt, (percent) => {
      send({ type: 'update-progress', ext: info.name, percent });
    });
    send({ type: 'update-progress', ext: info.name, percent: 100 });

    const zip = new AdmZip(tmpBrxt);
    // Keep the backup nested so a surviving backup cannot be scanned as an
    // installed extension. Build at the stable path: venv launchers embed it.
    backupRoot = await fs.mkdtemp(path.join(path.dirname(info.installDir), '.update-'));
    backupDir = path.join(backupRoot, 'package');
    await fs.cp(info.installDir, backupDir, { recursive: true, verbatimSymlinks: true });
    backupReady = true;
    safeExtractZip(zip, info.installDir);

    // Re-run uv sync to apply any dependency changes.
    // Async, not spawnSync: this runs on a background timer 8 s after launch,
    // and a synchronous child froze the whole app for up to two minutes (#88).
    const uvResult = await runProbe('uv', ['sync'], 120_000, { cwd: info.installDir });

    if (!uvResult.ok) {
      const detail =
        uvResult.stderr ||
        uvResult.stdout ||
        uvResult.error ||
        `exited with status ${uvResult.code}`;
      throw new Error(`uv sync failed: ${detail}`);
    }
  } catch (err) {
    error = (err as Error).message;
    if (backupReady && backupDir) {
      try {
        await fs.rm(info.installDir, { recursive: true, force: true });
        await fs.rename(backupDir, info.installDir);
      } catch (restoreError) {
        retainBackup = true;
        error += `. Could not restore the previous version: ${(restoreError as Error).message}. Its backup remains at ${backupDir}`;
      }
    }
  } finally {
    for (const directory of [tmpDir, retainBackup ? undefined : backupRoot]) {
      if (!directory) continue;
      try {
        await fs.rm(directory, { recursive: true, force: true });
      } catch {
        /* best-effort cleanup — ignore if the temp dir is already gone */
      }
    }
  }

  if (error !== undefined) {
    log.error(`[ExtensionUpdater] Failed to update ${info.name}:`, error);
    send({ type: 'update-error', ext: info.name, error });
    return false;
  }

  send({
    type: 'update-done',
    ext: info.name,
    displayName: info.displayName,
    to: info.latestVersion,
  });
  log.info(`[ExtensionUpdater] Updated ${info.name} → ${info.latestVersion}`);
  return true;
}

// ─── Public entry point ───────────────────────────────────────────────────────

let updateCheckInFlight: Promise<void> | undefined;

export function runExtensionUpdateCheck(): Promise<void> {
  // Startup and menu requests share the check so a failed update cannot roll
  // its older snapshot back over another check's committed package.
  if (!updateCheckInFlight) {
    updateCheckInFlight = checkInstalledExtensionUpdates().finally(() => {
      updateCheckInFlight = undefined;
    });
  }
  return updateCheckInFlight;
}

async function checkInstalledExtensionUpdates(): Promise<void> {
  // The directory is logged because it is now variable: when this check turns
  // out to have touched the wrong tree, the log says which tree it was.
  log.info(`[ExtensionUpdater] Starting extension update check in ${biorouterExtensionsDir()}`);

  const installed = await scanInstalledExtensions();
  if (installed.length === 0) {
    log.info('[ExtensionUpdater] No installed extensions found.');
    return;
  }

  // Notify all windows about updates via a push event
  const send = (evt: ExtensionUpdateEvent) => {
    BrowserWindow.getAllWindows().forEach((win) => {
      if (!win.isDestroyed()) win.webContents.send('extension-update-event', evt);
    });
  };

  let updatedCount = 0;
  for (const { manifest, installDir } of installed) {
    let updateInfo: ExtensionUpdateInfo | null = null;
    try {
      updateInfo = await checkExtensionUpdate(manifest, installDir);
    } catch (err) {
      // Network/parse error — skip silently (offline fallback)
      log.warn(`[ExtensionUpdater] Skipping ${manifest.name}:`, (err as Error).message);
      continue;
    }

    if (!updateInfo) continue;

    send({
      type: 'update-found',
      ext: updateInfo.name,
      displayName: updateInfo.displayName,
      from: updateInfo.currentVersion,
      to: updateInfo.latestVersion,
    });
    if (await applyUpdate(updateInfo, send)) updatedCount++;
  }

  send({ type: 'all-done', updatedCount });
  log.info(`[ExtensionUpdater] Done. ${updatedCount} extension(s) updated.`);
}

/**
 * Called from main.ts after the app window is ready. The delay is supplied by the
 * caller so every startup task's timing lives in one place — see
 * `utils/startupSchedule.ts`.
 */
export function scheduleExtensionUpdateCheck(delayMs = 15_000): void {
  setTimeout(() => {
    runExtensionUpdateCheck().catch((err) => log.error('[ExtensionUpdater] Unhandled error:', err));
  }, delayMs);
}
