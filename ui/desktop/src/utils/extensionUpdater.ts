/**
 * extensionUpdater.ts
 *
 * On startup, scans installed extensions in the Biorouter config directory's
 * `extensions/` folder, reads each manifest.json, decides which release (if
 * any) it may move to, and auto-updates outdated ones in the background.
 *
 * Which release is decided by `extensionReleaseSource.ts`: an extension BAAM
 * lists follows its registry row, anything else takes exactly `<name>.brxt`
 * from `releases/latest`, and nothing is installed without the SHA-256 digest
 * GitHub publishes for the asset. This module downloads it, checks the bytes
 * against that digest and the bundle's manifest against the install BEFORE
 * opening anything over the install, then extracts it with the backup restore
 * below, and refuses (restoring the backup) when the `manifest.json` left on
 * disk is not the one it checked.
 *
 * Skips silently on network errors so offline users are unaffected.
 */

import * as path from 'path';
import * as fs from 'fs/promises';
import { createHash } from 'crypto';
import { BrowserWindow } from 'electron';
import AdmZip from 'adm-zip';
import { assertDistinctZipEntries, extractedFileMatchesEntry, safeExtractZip } from './safeZip';
import log from './logger';
import { runProbe } from './dependencyChecker';
import { biorouterExtensionsDir as sharedExtensionsDir } from './biorouterPaths';
import { isRegistryDocument, type RegistryFetchResult } from './registryCache';
import {
  checkBundleManifest,
  chooseExtensionRelease,
  cleanVersion,
  type InstalledManifest,
} from './extensionReleaseSource';
import bundledRegistry from '../components/baam/registry.fallback.json';

// ─── Types ────────────────────────────────────────────────────────────────────

interface ExtensionManifest extends InstalledManifest {
  display_name: string;
  description: string;
}

interface ExtensionUpdateInfo {
  name: string;
  displayName: string;
  currentVersion: string;
  latestVersion: string;
  downloadUrl: string;
  /** The release asset's file name, for the notices. */
  assetName: string;
  /** 64 lowercase hex digits: the digest GitHub published for the asset. */
  sha256: string;
  /** The installed manifest the bundle is held to. */
  installed: ExtensionManifest;
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

/**
 * Where the BAAM catalogue comes from. `main.ts` registers the live one (the
 * same document and last-good cache `registry:fetch` serves the Browse modal)
 * before the startup check is scheduled; the menu's own checks call
 * `runExtensionUpdateCheck()` with no argument and read the same loader.
 */
export type ExtensionUpdateRegistryLoader = () => Promise<RegistryFetchResult | null | undefined>;

let registryLoader: ExtensionUpdateRegistryLoader | undefined;

export function setExtensionUpdateRegistry(
  loader: ExtensionUpdateRegistryLoader | undefined
): void {
  registryLoader = loader;
}

/**
 * The catalogue for one check: the registered loader's document, or the
 * snapshot bundled with the app when there is no loader or it has nothing to
 * give (offline with no cache, a page that is not a catalogue). Falling back
 * matters: with no catalogue at all, a BAAM-listed extension would be treated
 * as unlisted and asked of `releases/latest`, which is exactly what its row
 * exists to prevent. A stale snapshot can never move an install backwards,
 * because only a newer version is ever installed.
 */
async function loadUpdateRegistry(): Promise<unknown> {
  if (registryLoader) {
    try {
      const result = await registryLoader();
      if (result && isRegistryDocument(result.registry)) return result.registry;
      log.warn(
        '[ExtensionUpdater] The marketplace catalog could not be read; using the one bundled with the app.',
        result?.error ?? ''
      );
    } catch (err) {
      log.warn(
        '[ExtensionUpdater] The marketplace catalog could not be read; using the one bundled with the app.',
        (err as Error).message
      );
    }
  }
  return bundledRegistry;
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

/**
 * The largest bundle the updater reads. A bundle is held in memory whole so its
 * digest can be checked before anything is written, so an unbounded download is
 * unbounded memory. Extension bundles are megabytes; this leaves room for one
 * that ships a runtime.
 */
export const MAX_BUNDLE_BYTES = 256 * 1024 * 1024;

function tooLarge(maxBytes: number): Error {
  return new Error(
    `The download is larger than ${Math.round(maxBytes / (1024 * 1024))} MB, more than any extension bundle needs. Nothing was changed.`
  );
}

/**
 * Download a URL into memory, tracking progress, refusing more than `maxBytes`:
 * by the stated size before reading, and by the bytes received while reading,
 * since the stated size is the server's word.
 */
export async function downloadBytes(
  url: string,
  onProgress: (percent: number) => void,
  maxBytes = MAX_BUNDLE_BYTES
): Promise<Buffer> {
  const res = await fetch(url, {
    headers: { 'User-Agent': 'Biorouter-ExtensionUpdater/1.0' },
  });
  if (!res.ok) throw new Error(`Download failed: HTTP ${res.status}`);

  const total = Number(res.headers.get('content-length') || 0);
  if (total > maxBytes) {
    await res.body?.cancel().catch(() => {});
    throw tooLarge(maxBytes);
  }
  let received = 0;
  const chunks: Uint8Array[] = [];

  const reader = res.body?.getReader();
  if (!reader) throw new Error('No response body');

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    received += value.length;
    if (received > maxBytes) {
      await reader.cancel().catch(() => {});
      throw tooLarge(maxBytes);
    }
    chunks.push(value);
    if (total > 0) onProgress(Math.round((received / total) * 100));
  }

  return Buffer.concat(chunks);
}

/**
 * Releases this run of the app refused for what the published bundle IS (its
 * manifest, or entries that cannot be extracted safely), keyed by the install
 * and the asset's digest. The same bytes are refused the same way at every
 * check, so the notice is shown once per run rather than at each startup and
 * menu check; a new release has a new digest and is judged afresh. A digest
 * mismatch or a network failure is not recorded: those can pass on a retry.
 */
const refusedReleases = new Set<string>();

function refusalKey(installDir: string, sha256: string): string {
  return `${installDir}\0${sha256}`;
}

/** A refusal that comes from the published bytes themselves. */
class BundleRefusal extends Error {}

/** The bundle's own `manifest.json`, parsed, or null when it has none it can read. */
function readBundleManifest(zip: AdmZip): unknown {
  const entry = zip.getEntry('manifest.json');
  if (!entry || entry.isDirectory) return null;
  try {
    return JSON.parse(entry.getData().toString('utf8'));
  } catch {
    return null;
  }
}

// ─── Core update flow ─────────────────────────────────────────────────────────

interface InstalledExtension {
  manifest: ExtensionManifest;
  installDir: string;
  /** The folder's name under `extensions/`, which can differ from the manifest's name. */
  folderName: string;
}

async function scanInstalledExtensions(): Promise<InstalledExtension[]> {
  const extensionsDir = biorouterExtensionsDir();
  try {
    const entries = await fs.readdir(extensionsDir, { withFileTypes: true });
    const results: InstalledExtension[] = [];
    for (const entry of entries) {
      if (!entry.isDirectory()) continue;
      const manifestPath = path.join(extensionsDir, entry.name, 'manifest.json');
      try {
        const raw = await fs.readFile(manifestPath, 'utf8');
        const manifest = JSON.parse(raw) as ExtensionManifest;
        // No repository is needed for an extension BAAM lists: its row says
        // where releases come from. `chooseExtensionRelease` skips an unlisted
        // one without a GitHub repository.
        if (
          typeof manifest.name === 'string' &&
          manifest.name &&
          typeof manifest.version === 'string'
        ) {
          results.push({
            manifest,
            installDir: path.join(extensionsDir, entry.name),
            folderName: entry.name,
          });
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
  { manifest, installDir, folderName }: InstalledExtension,
  registry: unknown
): Promise<ExtensionUpdateInfo | null> {
  let decision;
  try {
    decision = await chooseExtensionRelease({
      folderName,
      manifest,
      registry,
      fetchJson: (url) => fetchJson<unknown>(url),
    });
  } catch (err) {
    log.warn(
      `[ExtensionUpdater] Cannot fetch release for ${manifest.name}:`,
      (err as Error).message
    );
    return null;
  }

  switch (decision.kind) {
    case 'up-to-date':
      return null;
    case 'skip':
      log.info(`[ExtensionUpdater] Not checking ${folderName}: ${decision.reason}.`);
      return null;
    case 'refuse':
      log.warn(`[ExtensionUpdater] Not updating ${manifest.name}: ${decision.reason}.`);
      return null;
    case 'update':
      break;
  }

  const { release } = decision;
  if (refusedReleases.has(refusalKey(installDir, release.sha256))) {
    log.info(
      `[ExtensionUpdater] Not updating ${manifest.name}: ${release.tag}/${release.assetName} was refused earlier in this run.`
    );
    return null;
  }
  log.info(
    `[ExtensionUpdater] ${manifest.name} ${manifest.version} → ${release.version} ` +
      `from ${release.from === 'registry' ? 'its marketplace entry' : 'the latest release'} ` +
      `(${release.tag}/${release.assetName}${release.prerelease ? ', a prerelease' : ''})`
  );
  return {
    name: manifest.name,
    displayName: manifest.display_name,
    currentVersion: cleanVersion(manifest.version),
    latestVersion: release.version,
    downloadUrl: release.downloadUrl,
    assetName: release.assetName,
    sha256: release.sha256,
    installed: manifest,
    installDir,
  };
}

async function applyUpdate(
  info: ExtensionUpdateInfo,
  send: (e: ExtensionUpdateEvent) => void
): Promise<boolean> {
  let backupRoot: string | undefined;
  let backupDir: string | undefined;
  let backupReady = false;
  let retainBackup = false;
  let error: string | undefined;

  try {
    send({
      type: 'update-start',
      ext: info.name,
      displayName: info.displayName,
      from: info.currentVersion,
      to: info.latestVersion,
    });

    // Download
    const bytes = await downloadBytes(info.downloadUrl, (percent) => {
      send({ type: 'update-progress', ext: info.name, percent });
    });
    send({ type: 'update-progress', ext: info.name, percent: 100 });

    // Every check below runs before the install is touched, so a refused bundle
    // leaves nothing to restore. The digest first, before the archive is even
    // opened: until it matches, these bytes are not the release.
    const actual = createHash('sha256').update(bytes).digest('hex');
    if (actual !== info.sha256) {
      throw new Error(
        `The download does not match the checksum GitHub published for ${info.assetName}. Nothing was changed.`
      );
    }
    let zip: AdmZip;
    try {
      zip = new AdmZip(bytes);
      // Every entry first: one outside the install, or two that could name one
      // file on disk (`manifest.json` beside `MANIFEST.JSON`, `./manifest.json`
      // or `manifeſt.json`, where the lookup below would check one and the
      // extraction install the other). `safeExtractZip` checks again, but
      // refusing here means a hostile bundle never starts the backup and
      // restore. A name rule cannot know every disk, so the extracted manifest
      // is also read back below.
      assertDistinctZipEntries(zip);
    } catch (err) {
      throw new BundleRefusal(`${(err as Error).message}. Nothing was changed.`);
    }
    const mismatch = checkBundleManifest(
      info.installed,
      readBundleManifest(zip),
      info.latestVersion
    );
    if (mismatch) throw new BundleRefusal(`${mismatch} Nothing was changed.`);

    // Keep the backup nested so a surviving backup cannot be scanned as an
    // installed extension. Build at the stable path: venv launchers embed it.
    backupRoot = await fs.mkdtemp(path.join(path.dirname(info.installDir), '.update-'));
    backupDir = path.join(backupRoot, 'package');
    await fs.cp(info.installDir, backupDir, { recursive: true, verbatimSymlinks: true });
    backupReady = true;
    safeExtractZip(zip, info.installDir);
    // The layer that mirrors no disk's rules. Whatever the extraction left as
    // `manifest.json` steers every later check (its name picks the registry
    // row, its entry point is what the next bundle is held to, its repository
    // is where an unlisted extension's releases come from), so it must be the
    // bytes checked above, and not another entry the disk merged into it.
    if (!extractedFileMatchesEntry(zip, info.installDir, 'manifest.json')) {
      throw new BundleRefusal(
        'The manifest.json the bundle left on disk is not the one that was checked, so another of its entries names the same file. The previous version was restored.'
      );
    }

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
    if (err instanceof BundleRefusal) refusedReleases.add(refusalKey(info.installDir, info.sha256));
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
    if (backupRoot && !retainBackup) {
      try {
        await fs.rm(backupRoot, { recursive: true, force: true });
      } catch {
        /* best-effort cleanup: the backup dir may already be gone */
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
  const registry = await loadUpdateRegistry();

  // Notify all windows about updates via a push event
  const send = (evt: ExtensionUpdateEvent) => {
    BrowserWindow.getAllWindows().forEach((win) => {
      if (!win.isDestroyed()) win.webContents.send('extension-update-event', evt);
    });
  };

  let updatedCount = 0;
  for (const extension of installed) {
    let updateInfo: ExtensionUpdateInfo | null = null;
    try {
      updateInfo = await checkExtensionUpdate(extension, registry);
    } catch (err) {
      // Network/parse error — skip silently (offline fallback)
      log.warn(`[ExtensionUpdater] Skipping ${extension.manifest.name}:`, (err as Error).message);
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
