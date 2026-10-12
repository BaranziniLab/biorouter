// @vitest-environment node

/**
 * Settings > App > Version showed 39.8.10 in a development launch: the dev launcher hands Electron
 * the built `.vite/build/main.js` directly, there is no `package.json` beside it, and
 * `app.getVersion()` then answers with Electron's own version. These pin the replacement: the
 * main process answers Biorouter's version whatever Electron reports.
 */
import { describe, it, expect, vi } from 'vitest';
import { readFileSync } from 'fs';
import * as path from 'path';
import {
  BUNDLED_BIOROUTER_VERSION,
  adoptBiorouterVersion,
  biorouterVersion,
  registerAppVersionIpc,
} from './biorouterVersion';

const ELECTRON_VERSION = '39.8.10';

const packageJsonVersion = (
  JSON.parse(readFileSync(path.resolve(__dirname, '../../package.json'), 'utf8')) as {
    version: string;
  }
).version;

function appReporting(version: string) {
  return { getVersion: vi.fn(() => version), setVersion: vi.fn() };
}

describe('biorouterVersion', () => {
  it('is built from ui/desktop/package.json, the version release.sh bumps', () => {
    expect(BUNDLED_BIOROUTER_VERSION).toBe(packageJsonVersion);
    expect(BUNDLED_BIOROUTER_VERSION).not.toBe(ELECTRON_VERSION);
  });

  it('answers Biorouter’s version when Electron reports its own', () => {
    expect(biorouterVersion(appReporting(ELECTRON_VERSION), ELECTRON_VERSION)).toBe(
      BUNDLED_BIOROUTER_VERSION
    );
  });

  it('keeps what a packaged app reports', () => {
    expect(biorouterVersion(appReporting('1.93.0'), ELECTRON_VERSION)).toBe('1.93.0');
  });

  it('answers Biorouter’s version when nothing is reported', () => {
    expect(biorouterVersion(appReporting(''), ELECTRON_VERSION)).toBe(BUNDLED_BIOROUTER_VERSION);
  });

  it('keeps any report outside Electron (tests, plain Node)', () => {
    expect(biorouterVersion(appReporting('0.0.0-test'), undefined)).toBe('0.0.0-test');
  });
});

describe('adoptBiorouterVersion', () => {
  it('sets Electron’s own answer to Biorouter’s, so electron-updater compares Biorouter versions', () => {
    const app = appReporting(ELECTRON_VERSION);
    expect(adoptBiorouterVersion(app, ELECTRON_VERSION)).toBe(BUNDLED_BIOROUTER_VERSION);
    expect(app.setVersion).toHaveBeenCalledWith(BUNDLED_BIOROUTER_VERSION);
  });

  it('leaves a packaged app’s version alone', () => {
    const app = appReporting('1.93.0');
    expect(adoptBiorouterVersion(app, ELECTRON_VERSION)).toBe('1.93.0');
    expect(app.setVersion).not.toHaveBeenCalled();
  });

  it('still answers Biorouter’s version when Electron has no setVersion', () => {
    expect(adoptBiorouterVersion({ getVersion: () => ELECTRON_VERSION }, ELECTRON_VERSION)).toBe(
      BUNDLED_BIOROUTER_VERSION
    );
  });
});

describe('get-app-version IPC', () => {
  function register(app: { getVersion(): string }) {
    const handlers = new Map<string, (event: { returnValue?: unknown }) => void>();
    const ipcMain = {
      on: vi.fn((channel: string, handler: (event: { returnValue?: unknown }) => void) => {
        handlers.set(channel, handler);
        return ipcMain;
      }),
    };
    registerAppVersionIpc(ipcMain as never, app, ELECTRON_VERSION);
    return (channel: string) => {
      const event: { returnValue?: unknown } = {};
      handlers.get(channel)?.(event);
      return event.returnValue;
    };
  }

  it('returns Biorouter’s version to the renderer in a development launch', () => {
    const sendSync = register(appReporting(ELECTRON_VERSION));
    expect(sendSync('get-app-version')).toBe(BUNDLED_BIOROUTER_VERSION);
  });

  it('returns the packaged app’s version in a packaged launch', () => {
    const sendSync = register(appReporting('1.93.0'));
    expect(sendSync('get-app-version')).toBe('1.93.0');
  });
});
