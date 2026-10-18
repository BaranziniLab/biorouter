import { autoUpdater, UpdateInfo } from 'electron-updater';
import {
  autoUpdater as squirrelMac,
  BrowserWindow,
  ipcMain,
  nativeImage,
  Tray,
  shell,
  app,
  dialog,
  Menu,
  MenuItemConstructorOptions,
  Notification,
} from 'electron';
import * as path from 'path';
import * as fs from 'fs/promises';
import { writeFileSync } from 'fs';
import log from './logger';
import {
  assistedUpdateInstructions,
  assistedUpdateKind,
  updateRequiresQuitFirst,
} from './assistedUpdateInstructions';
import { githubUpdater } from './githubUpdater';
import { biorouterVersion } from './biorouterVersion';
import { loadRecentDirs } from './recentDirs';
import { scheduleUpdateChecks, type AutomaticUpdateCheckReason } from './updateCheckSchedule';
import { MacInstallCoordinator } from './macUpdateInstall';
import {
  describeUpdateDownloadError,
  installResumableUpdaterDownloads,
} from './resumableUpdaterDownloads';
import { electronNetOpenPart } from './electronNetTransport';
import {
  initialUpdaterState,
  isReadyPhase,
  reduceUpdaterEvent,
  type UpdatePhase,
  type UpdaterState,
} from './updaterState';

let updateAvailable = false;
let trayRef: Tray | null = null;
let trayMenu: Menu | null = null;
let isUsingGitHubFallback = false;
let githubUpdateInfo: {
  latestVersion?: string;
  downloadUrl?: string;
  releaseUrl?: string;
  downloadPath?: string;
  extractedPath?: string;
} = {};

// The view every window shows, folded by the SAME reducer the renderer runs, so
// a window that opens late (`get-update-state`) recovers exactly what a window
// that was open all along is showing. This used to be a second, hand-written
// copy of the reducer, which is how the two could disagree about what an event
// means.
let viewState: UpdaterState = initialUpdaterState;

/** What `get-update-state` returns: null until the updater has said anything. */
function updateSnapshot(): {
  updateAvailable: boolean;
  latestVersion?: string;
  status: UpdatePhase;
  percent: number;
  usingFallback: boolean;
  error?: string;
} | null {
  if (viewState.phase === 'idle') return null;
  return {
    updateAvailable: viewState.phase === 'available' || isReadyPhase(viewState.phase),
    latestVersion: viewState.latestVersion,
    status: viewState.phase,
    percent: viewState.percent,
    usingFallback: viewState.usingFallback,
    error: viewState.error,
  };
}

// The download electron-updater is running for us, if any. `autoDownload` is
// off: a download started by electron-updater itself could neither be kept
// from repeating (every three-hourly check re-downloaded and re-staged the same
// update) nor told apart from a failed check when it errored.
/** Resolves to the failure message, or null once the download is done. */
let downloadInFlight: Promise<string | null> | null = null;
/**
 * True from the moment a download is requested until it settles. Separate from
 * `downloadInFlight` because electron-updater may emit the download's `error`
 * before `downloadUpdate()` has returned the promise to assign.
 */
let downloading = false;

// macOS only: Squirrel.Mac's staging and the quit that installs it. See
// macUpdateInstall.ts for why "downloaded" is not "ready to install".
const macInstall = new MacInstallCoordinator({
  quitAndInstall: () => autoUpdater.quitAndInstall(false, true),
  emit: (event, data) => sendStatusToWindow(event, data),
  ensureWindow: () => openWindowIfNone(),
  log: (message) => log.info(`[update] ${message}`),
});

// Track last reported progress to prevent backward jumps
let lastReportedProgress = 0;

// Track if IPC handlers have been registered
let ipcUpdateHandlersRegistered = false;

// Register IPC handlers (only once)
export function registerUpdateIpcHandlers() {
  if (ipcUpdateHandlersRegistered) {
    return;
  }

  log.info('Registering update IPC handlers...');
  ipcUpdateHandlersRegistered = true;

  // IPC handlers for renderer process
  ipcMain.handle('check-for-updates', async () => {
    const currentVersion = autoUpdater.currentVersion?.version || biorouterVersion(app);
    const checkStartTime = Date.now();

    try {
      log.info('=== MANUAL UPDATE CHECK INITIATED ===');
      log.info(`Manual check for updates requested at ${new Date().toISOString()}`);
      log.info(`Current version: ${currentVersion}`);

      // Reset state for new update check
      isUsingGitHubFallback = false;
      githubUpdateInfo = {};
      lastReportedProgress = 0; // Reset progress tracking

      // Ensure auto-updater is properly initialized
      if (!autoUpdater.currentVersion) {
        log.error('Auto-updater currentVersion is null/undefined');
        throw new Error('Auto-updater not initialized. Restart Biorouter.');
      }

      log.info(
        `About to check for updates with currentVersion: ${JSON.stringify(autoUpdater.currentVersion)}`
      );
      log.info(`Feed URL: ${autoUpdater.getFeedURL()}`);

      const result = await autoUpdater.checkForUpdates();
      const duration = Date.now() - checkStartTime;
      log.info(`=== MANUAL UPDATE CHECK COMPLETED in ${duration}ms ===`);
      log.info('Auto-updater checkForUpdates result:', result);

      return {
        updateInfo: result?.updateInfo,
        error: null,
      };
    } catch (error) {
      const duration = Date.now() - checkStartTime;
      log.error(`=== MANUAL UPDATE CHECK FAILED after ${duration}ms ===`);
      log.error('Error checking for updates:', error);
      log.error('Manual check error details:', {
        message: error instanceof Error ? error.message : 'Unknown error',
        stack: error instanceof Error ? error.stack : 'No stack',
        name: error instanceof Error ? error.name : 'Unknown',
        code:
          error instanceof Error && 'code' in error
            ? (error as Error & { code: unknown }).code
            : undefined,
        toString: error?.toString(),
      });

      // If electron-updater fails, try GitHub API fallback
      if (
        error instanceof Error &&
        (error.message.includes('HttpError: 404') ||
          error.message.includes('ERR_CONNECTION_REFUSED') ||
          error.message.includes('ENOTFOUND') ||
          error.message.includes('No published versions'))
      ) {
        log.info('Using GitHub API fallback in check-for-updates...');
        log.info('Manual fallback triggered by error:', error.message);
        isUsingGitHubFallback = true;

        try {
          const result = await githubUpdater.checkForUpdates();

          if (result.error) {
            return {
              updateInfo: null,
              error: result.error,
            };
          }

          // Store GitHub update info
          if (result.updateAvailable) {
            githubUpdateInfo = {
              latestVersion: result.latestVersion,
              downloadUrl: result.downloadUrl,
              releaseUrl: result.releaseUrl,
            };

            updateAvailable = true;
            updateTrayIcon(true);
            sendStatusToWindow('update-available', { version: result.latestVersion });

            // Auto-download for GitHub fallback (matching autoDownload behavior)
            log.info('Auto-downloading update via GitHub fallback...');
            await githubAutoDownload(result.downloadUrl!, result.latestVersion!, 'manual check');
          } else {
            clearUpdateAvailabilityUnlessDownloaded();
            sendStatusToWindow('update-not-available', {
              version: autoUpdater.currentVersion.version,
            });
          }

          return {
            updateInfo: null,
            error: null,
          };
        } catch (fallbackError) {
          log.error('GitHub fallback also failed:', fallbackError);
          return {
            updateInfo: null,
            error: 'Could not check for updates. Check your internet connection.',
          };
        }
      }

      return {
        updateInfo: null,
        error: error instanceof Error ? error.message : 'Unknown error',
      };
    }
  });

  ipcMain.handle('download-update', async () => {
    try {
      if (isUsingGitHubFallback && githubUpdateInfo.downloadUrl && githubUpdateInfo.latestVersion) {
        log.info('Using GitHub fallback for download...');
        lastReportedProgress = 0; // Reset progress tracking

        const result = await githubUpdater.downloadUpdate(
          githubUpdateInfo.downloadUrl,
          githubUpdateInfo.latestVersion,
          (percent) => {
            // Only send if progress increased (monotonic)
            if (percent > lastReportedProgress) {
              lastReportedProgress = percent;
              sendStatusToWindow('download-progress', { percent });
            }
          }
        );

        if (result.success && result.downloadPath) {
          githubUpdateInfo.downloadPath = result.downloadPath;
          githubUpdateInfo.extractedPath = result.extractedPath;
          sendStatusToWindow('update-downloaded', { version: githubUpdateInfo.latestVersion });
          return { success: true, error: null };
        } else {
          const errorMsg = result.error || 'Download failed';
          throw new Error(errorMsg);
        }
      } else {
        // electron-updater, through the same door as the automatic download, so
        // a second press joins the running download and a failure is reported
        // once.
        const version = viewState.latestVersion;
        if (!version) throw new Error('No update to download. Check for updates first.');
        const error = await startUpdateDownload(version);
        return error ? { success: false, error } : { success: true, error: null };
      }
    } catch (error) {
      log.error('Error downloading update:', error);
      return {
        success: false,
        error: error instanceof Error ? error.message : 'Unknown error',
      };
    }
  });

  ipcMain.handle('install-update', async () => {
    // Decided by WHO downloaded the update, not by `isUsingGitHubFallback`.
    // That flag used to stay set for the life of the process after a single
    // failed check, and then a macOS update electron-updater had downloaded
    // was "installed" by the assisted path, which threw "Update file path not
    // found" into a renderer that never looked: Restart & update did nothing.
    if (process.platform === 'darwin' && macInstall.pendingVersion !== null) {
      return macInstall.requestInstall();
    }
    if (githubUpdateInfo.downloadPath) {
      // For GitHub fallback, we need to handle the installation differently
      log.info('Installing update from GitHub fallback...');

      try {
        // ⚠ The downloaded FILE, never `extractedPath`. Nothing is extracted any
        // more: `extractedPath` is the Downloads folder itself, and preferring it
        // classified a downloaded Setup.exe by the folder's name, so Windows got
        // the zip instructions and the "Open Folder Only" button that the
        // installer must never be offered (see updateRequiresQuitFirst).
        const updatePath = githubUpdateInfo.downloadPath;

        if (!updatePath) {
          throw new Error('Update file path not found. Download the update first.');
        }

        // Check if the update path exists
        try {
          await fs.access(updatePath);
        } catch {
          throw new Error('Update file not found. Download the update first.');
        }

        // The steps depend on WHICH artifact was downloaded, not just the
        // platform: Windows now publishes an installer and a zip, and the
        // updater prefers the installer. See assistedUpdateInstructions.ts.
        const detail = assistedUpdateInstructions(updatePath, process.platform);

        // ⚠ "Open Folder Only" leaves Biorouter RUNNING, and for a Squirrel
        // installer that is one double-click from a broken install: Squirrel
        // removes the app directory before checking whether anything is using
        // it, so it deletes Update.exe, packages\ and the root stub, then
        // throws. Both shortcuts survive pointing at a file that is gone.
        // Measured on Windows, 1.91.0. The instruction text says to quit, but an
        // option that is reasonable to pick and cannot work should not be there.
        const mustQuitFirst = updateRequiresQuitFirst(
          assistedUpdateKind(updatePath, process.platform)
        );
        const buttons = mustQuitFirst
          ? ['Open Folder & Quit', 'Cancel']
          : ['Open Folder & Quit', 'Open Folder Only', 'Cancel'];
        const cancelId = buttons.length - 1;

        const dialogResult = (await dialog.showMessageBox({
          type: 'info',
          title: 'Update ready to install',
          message: `Version ${githubUpdateInfo.latestVersion} is ready to install.`,
          detail,
          buttons,
          defaultId: 0,
          cancelId,
        })) as unknown as { response: number };

        if (dialogResult.response === 0) {
          // Open folder and quit app for easy replacement
          shell.showItemInFolder(updatePath);
          setTimeout(() => {
            app.quit();
          }, 1500); // Give user time to see the folder open
        } else if (!mustQuitFirst && dialogResult.response === 1) {
          // Just open folder, don't quit
          shell.showItemInFolder(updatePath);
        }
        // response === 2 is Cancel
        return { ok: true };
      } catch (error) {
        log.error('Error installing GitHub update:', error);
        return { ok: false, error: error instanceof Error ? error.message : String(error) };
      }
    }
    if (process.platform !== 'darwin' && isReadyPhase(viewState.phase)) {
      // electron-updater downloaded it on a platform without Squirrel.Mac.
      autoUpdater.quitAndInstall(false, true);
      return { ok: true };
    }
    return { ok: false, error: 'No update is ready to install yet. Check for updates first.' };
  });

  ipcMain.handle('get-current-version', () => {
    return autoUpdater.currentVersion.version;
  });

  ipcMain.handle('get-update-state', () => {
    return updateSnapshot();
  });

  ipcMain.handle('is-using-github-fallback', () => {
    return isUsingGitHubFallback;
  });
}

// Configure auto-updater
// Guards against a second `setupAutoUpdater()` installing a duplicate 3-hour
// interval and a duplicate set of `autoUpdater.on(...)` listeners — which would
// double every progress tick and fire two notifications per downloaded update.
// Mirrors `ipcUpdateHandlersRegistered` above.
let autoUpdaterConfigured = false;
let cancelScheduledChecks: (() => void) | null = null;

/** Stop the periodic update timer (app shutdown, tests). */
export function stopScheduledUpdateChecks(): void {
  cancelScheduledChecks?.();
  cancelScheduledChecks = null;
}

export function setupAutoUpdater(tray?: Tray) {
  if (tray) {
    trayRef = tray;
  }

  if (autoUpdaterConfigured) {
    log.info('Auto-updater already configured; skipping duplicate setup.');
    return;
  }
  autoUpdaterConfigured = true;

  log.info('Setting up auto-updater...');
  log.info(`Current app version: ${biorouterVersion(app)}`);
  log.info(`Platform: ${process.platform}, Arch: ${process.arch}`);
  log.info(`ENABLE_DEV_UPDATES: ${process.env.ENABLE_DEV_UPDATES}`);
  log.info(`App is packaged: ${app.isPackaged}`);
  log.info(`App path: ${app.getAppPath()}`);
  log.info(`Resources path: ${process.resourcesPath}`);

  // Set the feed URL. Defaults to the public GitHub releases of Biorouter, but
  // can be redirected to a generic (static-file) feed via
  // BIOROUTER_UPDATE_FEED_URL — used for controlled update testing and for
  // self-hosted / enterprise mirrors. The generic feed expects the same layout
  // electron-updater publishes: latest-mac.yml + the per-arch app zips.
  const feedOverride = process.env.BIOROUTER_UPDATE_FEED_URL;
  if (feedOverride) {
    // electron-updater resolves its provider from a config file (in packaged
    // builds: Resources/app-update.yml). electron-forge doesn't emit one, so
    // we synthesize a generic-provider config pointing at the override URL and
    // direct the updater at it via updateConfigPath. This is the robust path
    // for packaged builds (setFeedURL alone still triggers the missing-file
    // load). Used for controlled update testing and self-hosted/enterprise
    // mirrors; the feed expects electron-updater's layout (latest-mac.yml +
    // per-arch app zips).
    try {
      const cfgPath = path.join(app.getPath('userData'), 'biorouter-update-config.yml');
      writeFileSync(
        cfgPath,
        `provider: generic\nurl: ${feedOverride}\nupdaterCacheDirName: biorouter-updater\n`
      );
      autoUpdater.updateConfigPath = cfgPath;
      autoUpdater.forceDevUpdateConfig = true;
      log.info(`Update feed override active: ${feedOverride} (config: ${cfgPath})`);
    } catch (e) {
      log.error('Failed to apply update feed override:', e);
    }
  } else {
    const feedConfig = {
      provider: 'github' as const,
      owner: 'BaranziniLab',
      repo: 'biorouter',
      releaseType: 'release' as const,
    };
    log.info('Setting feed URL with config:', feedConfig);
    // Same reason as the override branch above: electron-updater reads its
    // config file (Resources/app-update.yml) at *download* time to resolve
    // `updaterCacheDirName`, even when the provider is set via setFeedURL().
    // electron-forge never emits that file, so a real GitHub update download
    // dies with `ENOENT … app-update.yml`. Synthesize a github-provider config
    // and point the updater at it so the download path always has a config to
    // read; setFeedURL is kept as a redundant, explicit provider source.
    try {
      const cfgPath = path.join(app.getPath('userData'), 'biorouter-update-config.yml');
      writeFileSync(
        cfgPath,
        `provider: github\nowner: ${feedConfig.owner}\nrepo: ${feedConfig.repo}\n` +
          `releaseType: ${feedConfig.releaseType}\nupdaterCacheDirName: biorouter-updater\n`
      );
      autoUpdater.updateConfigPath = cfgPath;
      log.info(`Wrote github update config: ${cfgPath}`);
    } catch (e) {
      log.error('Failed to write github update config:', e);
    }
    autoUpdater.setFeedURL(feedConfig);
  }

  // Log the feed URL after setting it
  try {
    const feedUrl = autoUpdater.getFeedURL();
    log.info(`Feed URL set to: ${feedUrl}`);
  } catch (e) {
    log.error('Error getting feed URL:', e);
  }

  // Configure auto-updater settings. Updates still download on their own, but
  // through startUpdateDownload (on `update-available`), not electron-updater's
  // autoDownload: see `downloadInFlight`.
  autoUpdater.autoDownload = false;
  autoUpdater.autoInstallOnAppQuit = true;

  // One dropped connection used to end the download (net::ERR_CONNECTION_RESET,
  // 1.93.0 → 1.93.1). Resume it instead. See resumableDownload.ts.
  if (
    installResumableUpdaterDownloads(autoUpdater, electronNetOpenPart(), (m) =>
      log.info(`[update] ${m}`)
    )
  ) {
    log.info('Update downloads resume after a dropped connection');
  }

  if (process.platform === 'darwin') {
    // Squirrel.Mac's OWN readiness, which is what Restart & update depends on.
    // Its errors also reach electron-updater's 'error' event, handled below.
    squirrelMac.on('update-downloaded', () => {
      if (macInstall.staged()) showUpdateReadyNotification();
    });
  }

  // Enable updates in development mode for testing
  if (process.env.ENABLE_DEV_UPDATES === 'true') {
    log.info('Enabling dev updates config');
    autoUpdater.forceDevUpdateConfig = true;
  }

  // Additional debugging for release builds
  if (app.isPackaged) {
    log.info('App is packaged - this is a release build');
    // Try to get more info about the updater configuration
    try {
      log.info(`Auto-updater channel: ${autoUpdater.channel}`);
      log.info(`Auto-updater allowPrerelease: ${autoUpdater.allowPrerelease}`);
      log.info(`Auto-updater allowDowngrade: ${autoUpdater.allowDowngrade}`);
    } catch (e) {
      log.error('Error getting auto-updater properties:', e);
    }
  } else {
    log.info('App is not packaged - this is a development build');
  }

  // Set logger
  autoUpdater.logger = log;

  log.info('Auto-updater setup completed');

  let automaticCheckInFlight = false;
  const runAutomaticUpdateCheck = async (reason: AutomaticUpdateCheckReason) => {
    if (automaticCheckInFlight) return;
    automaticCheckInFlight = true;
    const checkStartTime = Date.now();
    const reasonLabel = reason.toUpperCase();
    log.info(`=== ${reasonLabel} UPDATE CHECK INITIATED ===`);
    log.info(`Checking for updates (${reason}) at ${new Date().toISOString()}`);
    log.info(`autoUpdater.currentVersion: ${JSON.stringify(autoUpdater.currentVersion)}`);
    log.info(`autoUpdater.getFeedURL(): ${autoUpdater.getFeedURL()}`);
    log.info(
      `Network online status: ${typeof navigator !== 'undefined' ? navigator.onLine : 'unknown'}`
    );

    // Set up a timeout warning for long-running checks
    const timeoutWarning = setTimeout(() => {
      log.warn(
        `Update check still in progress after 30 seconds (started at ${new Date(checkStartTime).toISOString()})`
      );
    }, 30000);

    const timeoutError = setTimeout(() => {
      log.error(
        `Update check appears stuck - no response after 60 seconds (started at ${new Date(checkStartTime).toISOString()})`
      );
    }, 60000);

    try {
      const result = await autoUpdater.checkForUpdates();
      const duration = Date.now() - checkStartTime;
      log.info(`=== ${reasonLabel} UPDATE CHECK COMPLETED in ${duration}ms ===`);
      log.info('Update check result:', result);
    } catch (err) {
      const error = err instanceof Error ? err : new Error(String(err));
      const duration = Date.now() - checkStartTime;
      log.error(`=== ${reasonLabel} UPDATE CHECK FAILED after ${duration}ms ===`);
      log.error(`Error checking for updates (${reason}):`, error);
      log.error('Error details:', {
        message: error.message,
        stack: error.stack,
        name: error.name,
        code: 'code' in error ? error.code : undefined,
      });

      // If electron-updater fails, try GitHub API as fallback
      if (
        error.message.includes('HttpError: 404') ||
        error.message.includes('ERR_CONNECTION_REFUSED') ||
        error.message.includes('ENOTFOUND') ||
        error.message.includes('No published versions')
      ) {
        log.info(`Using GitHub API fallback for ${reason} update check...`);
        log.info('Fallback triggered by error containing:', error.message);
        isUsingGitHubFallback = true;

        try {
          const result = await githubUpdater.checkForUpdates();
          if (result.error) {
            sendStatusToWindow('error', result.error);
          } else if (result.updateAvailable) {
            githubUpdateInfo = {
              latestVersion: result.latestVersion,
              downloadUrl: result.downloadUrl,
              releaseUrl: result.releaseUrl,
            };

            updateAvailable = true;
            updateTrayIcon(true);
            sendStatusToWindow('update-available', { version: result.latestVersion });

            // Deliberately NOT downloaded here. This is the fallback path, reached
            // because the normal updater errored — including on a transient DNS
            // failure — and it writes a several-hundred-megabyte installer into the
            // user's Downloads folder. Doing that unprompted, seconds after launch,
            // on a background timer the user never asked for, is the wrong default
            // (#88). The update is announced; the download happens when the user
            // acts on it, via the `download-update` IPC.
            log.info(
              `GitHub fallback found ${result.latestVersion} during the ${reason} check; ` +
                'waiting for the user before downloading.'
            );
          } else {
            clearUpdateAvailabilityUnlessDownloaded();
            sendStatusToWindow('update-not-available', {
              version: autoUpdater.currentVersion.version,
            });
          }
        } catch (fallbackError) {
          log.error(`GitHub fallback also failed during ${reason} check:`, fallbackError);
        }
      }
    } finally {
      clearTimeout(timeoutWarning);
      clearTimeout(timeoutError);
      automaticCheckInFlight = false;
    }
  };

  // Keep the canceller: without it the 3-hour interval can never be cleared.
  cancelScheduledChecks = scheduleUpdateChecks(runAutomaticUpdateCheck);

  // Handle update events
  autoUpdater.on('checking-for-update', () => {
    log.info('Auto-updater: Checking for update...');
    log.info(`Auto-updater: Feed URL during check: ${autoUpdater.getFeedURL()}`);
    lastReportedProgress = 0; // Reset progress tracking for new check
    sendStatusToWindow('checking-for-update');
  });

  autoUpdater.on('update-available', (info: UpdateInfo) => {
    log.info('Update available:', info);
    // electron-updater answered, so it is the path: a fallback chosen after an
    // earlier failed check no longer applies.
    isUsingGitHubFallback = false;
    updateAvailable = true;
    updateTrayIcon(true);
    sendStatusToWindow('update-available', info);
    void startUpdateDownload(info.version);
  });

  autoUpdater.on('update-not-available', (info: UpdateInfo) => {
    log.info('Update not available:', info);
    isUsingGitHubFallback = false;
    clearUpdateAvailabilityUnlessDownloaded();
    sendStatusToWindow('update-not-available', info);
  });

  autoUpdater.on('error', async (err) => {
    log.error('Error in auto-updater:', err);
    log.error('Auto-updater error details:', {
      message: err.message,
      stack: err.stack,
      name: err.name,
      code: 'code' in err ? err.code : undefined,
      toString: err.toString(),
    });

    // A failed download is reported once, by startUpdateDownload, which also
    // sees this error as its rejection. Treating it here as well sent it down
    // the check-failure path below, where ERR_CONNECTION_REFUSED switched a
    // macOS download to the assisted DMG fallback.
    if (downloading) return;
    // Squirrel.Mac staging or installing the update: the install failed, and
    // the person must see why rather than a Restart button that does nothing.
    if (process.platform === 'darwin' && macInstall.ownsErrors) {
      macInstall.fail(err.message);
      return;
    }

    // Check if this is a 404 error (missing update files) or connection error
    if (
      err.message.includes('HttpError: 404') ||
      err.message.includes('ERR_CONNECTION_REFUSED') ||
      err.message.includes('ENOTFOUND') ||
      err.message.includes('No published versions')
    ) {
      log.info('Falling back to GitHub API for update check...');
      log.info('Fallback triggered by error:', err.message);
      isUsingGitHubFallback = true;

      try {
        const result = await githubUpdater.checkForUpdates();

        if (result.error) {
          sendStatusToWindow('error', result.error);
        } else if (result.updateAvailable) {
          // Store GitHub update info
          githubUpdateInfo = {
            latestVersion: result.latestVersion,
            downloadUrl: result.downloadUrl,
            releaseUrl: result.releaseUrl,
          };

          updateAvailable = true;
          updateTrayIcon(true);
          sendStatusToWindow('update-available', { version: result.latestVersion });

          // Announced, not downloaded — see the note on the background check path.
          log.info(
            `GitHub fallback found ${result.latestVersion} after an updater error; ` +
              'waiting for the user before downloading.'
          );
        } else {
          clearUpdateAvailabilityUnlessDownloaded();
          sendStatusToWindow('update-not-available', {
            version: autoUpdater.currentVersion.version,
          });
        }
      } catch (fallbackError) {
        log.error('GitHub fallback also failed:', fallbackError);
        sendStatusToWindow('error', 'Unable to check for updates. Check your internet connection.');
      }
    } else {
      sendStatusToWindow('error', err.message);
    }
  });

  autoUpdater.on('download-progress', (progressObj) => {
    const roundedPercent = Math.round(progressObj.percent);

    // Only send progress if it increased (prevents backward jumps)
    if (roundedPercent > lastReportedProgress) {
      lastReportedProgress = roundedPercent;

      const log_message = `Download: ${roundedPercent}% (${progressObj.transferred}/${progressObj.total}) @ ${Math.round(progressObj.bytesPerSecond / 1024)} KB/s`;
      log.info(log_message);

      sendStatusToWindow('download-progress', {
        ...progressObj,
        percent: roundedPercent,
      });
    }
  });

  autoUpdater.on('update-downloaded', (info: UpdateInfo) => {
    log.info('Update downloaded:', info);
    if (process.platform === 'darwin') {
      // Not installable yet: Squirrel.Mac stages it first, and macInstall says
      // `update-downloaded` to the windows once it has.
      macInstall.downloaded(info.version);
    } else {
      sendStatusToWindow('update-downloaded', info);
    }

    // Test-only: deterministically trigger the one-click install without a GUI
    // click, so the full quitAndInstall → in-place swap → relaunch path can be
    // exercised in an automated update test. Gated behind BOTH the feed
    // override and an explicit flag, so it can never fire in production. It
    // goes through requestInstall, the path the Restart button takes.
    if (
      process.env.BIOROUTER_UPDATE_FEED_URL &&
      process.env.BIOROUTER_UPDATE_AUTO_INSTALL === '1'
    ) {
      log.info('[test] BIOROUTER_UPDATE_AUTO_INSTALL set. Installing update now');
      setTimeout(() => {
        if (process.platform === 'darwin') macInstall.requestInstall();
        else autoUpdater.quitAndInstall(false, true);
      }, 1500);
    }
  });
}

/** The update is staged; say so outside the app too. Clicking installs it. */
function showUpdateReadyNotification(): void {
  const version = macInstall.pendingVersion;
  if (!version) return;
  const notification = new Notification({
    title: 'Update ready',
    body: `Version ${version} will be installed when you quit Biorouter. Click to install now.`,
  });
  notification.on('click', () => macInstall.requestInstall());
  notification.show();
}

/**
 * Download the update electron-updater just found, unless that version is
 * already downloaded or downloading.
 *
 * ⚠ Without the version check every three-hourly check downloaded the same
 * update again and had Squirrel.Mac stage it again: ShipIt's log on a machine
 * that left an update waiting shows a fresh "install request" every three
 * hours, and each one re-opened the window in which Restart & update could
 * race a half-staged update.
 */
function startUpdateDownload(version: string): Promise<string | null> {
  if (downloadInFlight) {
    log.info(`[update] A download is already running; not starting one for ${version}`);
    return downloadInFlight;
  }
  if (process.platform === 'darwin' && macInstall.pendingVersion === version) {
    log.info(`[update] ${version} is already downloaded; not downloading it again`);
    return Promise.resolve(null);
  }
  lastReportedProgress = 0;
  downloading = true;
  const download: Promise<string | null> = Promise.resolve()
    .then(() => autoUpdater.downloadUpdate())
    .then(() => null)
    .catch((error: unknown) => {
      if (error instanceof Error && error.name === 'CancellationError') return null;
      log.error('[update] Update download failed:', error);
      const message = describeUpdateDownloadError(error);
      if (process.platform === 'darwin' && macInstall.ownsErrors) macInstall.fail(message);
      else sendStatusToWindow('error', message);
      return message;
    })
    .finally(() => {
      downloading = false;
      downloadInFlight = null;
    });
  downloadInFlight = download;
  return download;
}

/** A window to show an update failure in, when the install closed them all. */
function openWindowIfNone(): void {
  if (BrowserWindow.getAllWindows().length > 0) return;
  const recentDirs = loadRecentDirs();
  ipcMain.emit('create-chat-window', {}, undefined, recentDirs[0] ?? null);
}

interface UpdaterEvent {
  event: string;
  data?: unknown;
  /** Assisted GitHub download rather than electron-updater. */
  usingFallback?: boolean;
}

function sendStatusToWindow(event: string, data?: unknown) {
  // Keep the persisted snapshot in lockstep with every emitted event so the
  // renderer can recover state if it mounts late (get-update-state).
  viewState = reduceUpdaterEvent(viewState, { event, data, usingFallback: isUsingGitHubFallback });
  const windows = BrowserWindow.getAllWindows();
  windows.forEach((win) => {
    // Stamp the mode on EVERY event. The renderer decides between "downloading
    // in the background" (electron-updater does it itself) and "press Download"
    // (the assisted GitHub path, which waits for the user) purely from this, and
    // it previously only ever reached the renderer through the late-mount
    // snapshot -- so a renderer that was already mounted never learned it.
    win.webContents.send('updater-event', {
      event,
      data,
      usingFallback: isUsingGitHubFallback,
    } as UpdaterEvent);
  });
}

function clearUpdateAvailabilityUnlessDownloaded() {
  const hasDownloadedUpdate = isReadyPhase(viewState.phase);
  updateAvailable = hasDownloadedUpdate;
  updateTrayIcon(hasDownloadedUpdate);
}

// centralize GitHub fallback auto-download logic.
async function githubAutoDownload(
  downloadUrl: string,
  latestVersion: string,
  contextLabel = ''
): Promise<void> {
  // Reset progress tracking for new download
  lastReportedProgress = 0;

  try {
    const downloadResult = await githubUpdater.downloadUpdate(
      downloadUrl,
      latestVersion,
      (percent) => {
        // Only send if progress increased (monotonic)
        if (percent > lastReportedProgress) {
          lastReportedProgress = percent;
          sendStatusToWindow('download-progress', { percent });
        }
      }
    );

    if (downloadResult.success && downloadResult.downloadPath) {
      githubUpdateInfo.downloadPath = downloadResult.downloadPath;
      githubUpdateInfo.extractedPath = downloadResult.extractedPath;
      sendStatusToWindow('update-downloaded', { version: latestVersion });
    } else {
      log.error(
        `GitHub auto-download failed${contextLabel ? ` (${contextLabel})` : ''}:`,
        downloadResult.error
      );
    }
  } catch (downloadError) {
    log.error(
      `Error during GitHub auto-download${contextLabel ? ` (${contextLabel})` : ''}:`,
      downloadError
    );
  }
}

// What the tray currently shows, so an unchanged state costs nothing.
let lastAppliedTrayState: boolean | null = null;

export function resetTrayStateForTests(): void {
  lastAppliedTrayState = null;
}

function updateTrayIcon(hasUpdate: boolean, opts?: { force?: boolean }) {
  if (!trayRef) return;

  if (process.env.BIOROUTER_VERSION) {
    hasUpdate = false;
  }

  // Every update check called through here, including the "already up to date"
  // case that fires on launch and then every three hours forever. Each call
  // re-read a PNG off disk, re-decoded it, and rebuilt a ten-item native Menu
  // from scratch to arrive at exactly the state already on screen.
  if (!opts?.force && lastAppliedTrayState === hasUpdate) {
    return;
  }
  lastAppliedTrayState = hasUpdate;

  const isDev = !app.isPackaged;
  let iconPath: string;

  if (hasUpdate) {
    // Use icon with update indicator
    if (isDev) {
      iconPath = path.join(process.cwd(), 'src', 'images', 'iconTemplateUpdate.png');
    } else {
      iconPath = path.join(process.resourcesPath, 'images', 'iconTemplateUpdate.png');
    }
    trayRef.setToolTip('Biorouter - Update Available');
  } else {
    // Use normal icon
    if (isDev) {
      iconPath = path.join(process.cwd(), 'src', 'images', 'iconTemplate.png');
    } else {
      iconPath = path.join(process.resourcesPath, 'images', 'iconTemplate.png');
    }
    trayRef.setToolTip('Biorouter');
  }

  const icon = nativeImage.createFromPath(iconPath);
  if (process.platform === 'darwin') {
    // Mark as template for macOS to handle dark/light mode
    icon.setTemplateImage(true);
  }
  trayRef.setImage(icon);

  // Update tray menu when icon changes
  updateTrayMenu(hasUpdate);
}

// Function to open settings and scroll to update section
export function openUpdateSettings() {
  const windows = BrowserWindow.getAllWindows();
  if (windows.length > 0) {
    const mainWindow = windows[0];
    mainWindow.show();
    mainWindow.focus();
    // Send message to open settings and scroll to update section
    mainWindow.webContents.send('set-view', 'settings', 'update');
  }
}

// Export function to update tray menu
export function updateTrayMenu(hasUpdate: boolean) {
  if (!trayRef) return;

  // Helper: show any existing window, then navigate to a view.
  // If no windows are open, create one first.
  const showAndNavigate = (view: string) => {
    const windows = BrowserWindow.getAllWindows();
    if (windows.length === 0) {
      const recentDirs = loadRecentDirs();
      const openDir = recentDirs.length > 0 ? recentDirs[0] : null;
      ipcMain.emit('create-chat-window', {}, undefined, openDir);
      return;
    }
    windows.forEach((win) => {
      if (!win.isVisible()) win.show();
      win.focus();
    });
    windows[windows.length - 1].webContents.send('set-view', view);
  };

  const menuItems: MenuItemConstructorOptions[] = [];

  if (hasUpdate) {
    menuItems.push({ label: 'Update Available…', click: openUpdateSettings });
    menuItems.push({ type: 'separator' });
  }

  menuItems.push(
    { label: 'Home', click: () => showAndNavigate('') },
    { label: 'New Chat', click: () => showAndNavigate('') },
    { label: 'Settings', click: () => showAndNavigate('settings') },
    { type: 'separator' },
    { label: 'Extensions', click: () => showAndNavigate('extensions') },
    { label: 'Skills', click: () => showAndNavigate('skills') },
    { type: 'separator' },
    { label: 'Check for Updates', click: openUpdateSettings },
    { type: 'separator' },
    { label: 'Quit', click: () => app.quit() }
  );

  const contextMenu = Menu.buildFromTemplate(menuItems);
  trayMenu = contextMenu;

  // On macOS, setContextMenu would make the menu pop on both left- and right-click.
  // We want left-click to show the app windows and right-click to show the menu,
  // so we keep the menu in memory and pop it up manually via popUpTrayMenu().
  if (process.platform !== 'darwin') {
    trayRef.setContextMenu(contextMenu);
  }
}

// Pop up the most recently built tray menu (used for macOS right-click).
export function popUpTrayMenu() {
  if (trayRef && trayMenu) {
    trayRef.popUpContextMenu(trayMenu);
  }
}

// Export functions to manage tray reference
export function setTrayRef(tray: Tray) {
  trayRef = tray;
  // Update icon based on current update status
  updateTrayIcon(updateAvailable);
}

export function getUpdateAvailable(): boolean {
  return updateAvailable;
}
