import type { IpcMain } from 'electron';
import { version as bundledPackageVersion } from '../../package.json';

/**
 * Biorouter's version as this bundle was built: `ui/desktop/package.json`'s `"version"`, which
 * `scripts/release.sh bump` keeps equal to the Cargo workspace version. Vite inlines it into the
 * main bundle, so it holds however Electron was started.
 */
export const BUNDLED_BIOROUTER_VERSION: string = bundledPackageVersion;

/** The part of Electron's `app` this module reads. */
export interface VersionedApp {
  getVersion(): string;
}

/**
 * Biorouter's version, never Electron's.
 *
 * `app.getVersion()` answers from the loaded app's `package.json`. A packaged app and
 * `electron-forge start` both load `ui/desktop`, so they answer Biorouter's version. A launch that
 * hands Electron the built `.vite/build/main.js` directly (the agent shell's dev launcher does)
 * has no `package.json` beside it, and Electron then answers with its OWN version: Settings showed
 * 39.8.10, and the GitHub fallback compared release tags against it, so a dev launch was never
 * offered an update. That one answer is replaced; any other is kept.
 */
export function biorouterVersion(
  app: VersionedApp,
  electronVersion: string | undefined = process.versions.electron
): string {
  const reported = app.getVersion();
  if (!reported || (electronVersion !== undefined && reported === electronVersion)) {
    return BUNDLED_BIOROUTER_VERSION;
  }
  return reported;
}

/**
 * Make Electron's own answer Biorouter's, once at startup, so code that reads `app.getVersion()`
 * for itself agrees: electron-updater takes `currentVersion` from it when it is first used.
 * `app.setVersion` is what Electron's default app calls with a loaded `package.json`'s version;
 * it is missing from Electron's typings, so its absence leaves the reported version alone and
 * every Biorouter reader still goes through `biorouterVersion`.
 */
export function adoptBiorouterVersion(
  app: VersionedApp & { setVersion?: (version: string) => void },
  electronVersion: string | undefined = process.versions.electron
): string {
  const version = biorouterVersion(app, electronVersion);
  if (app.getVersion() !== version && typeof app.setVersion === 'function') {
    app.setVersion(version);
  }
  return version;
}

/** The renderer's synchronous `window.electron.getVersion()` (preload: `get-app-version`). */
export function registerAppVersionIpc(
  ipcMain: Pick<IpcMain, 'on'>,
  app: VersionedApp,
  electronVersion: string | undefined = process.versions.electron
): void {
  ipcMain.on('get-app-version', (event) => {
    event.returnValue = biorouterVersion(app, electronVersion);
  });
}
