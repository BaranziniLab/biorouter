// @vitest-environment node

/**
 * The updater's glue: which events lead to which download, which install, and
 * what the windows are told. Electron and electron-updater are fakes; the
 * module under test is the real `autoUpdater.ts`.
 *
 * Each case is a bug that shipped:
 *  - one failed check left `isUsingGitHubFallback` set for the life of the app,
 *    and Restart & update on macOS then went to the assisted path and did
 *    nothing;
 *  - every three-hourly check downloaded and staged the same update again;
 *  - a download error went down the check-failure path, where
 *    ERR_CONNECTION_REFUSED switched a macOS download to the DMG fallback;
 *  - a downloaded Windows Setup.exe was classified by the Downloads folder's
 *    name and offered "Open Folder Only".
 */
import { EventEmitter } from 'events';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type Handler = (...args: unknown[]) => unknown;

const h = vi.hoisted(() => ({
  handlers: {} as Record<string, Handler>,
  sent: [] as Array<{ event: string; data?: unknown; usingFallback?: boolean }>,
  userData: '',
  scheduled: null as null | ((reason: string) => Promise<void>),
  dialogResponse: 2,
  dialogCalls: [] as Array<{ buttons: string[]; detail: string }>,
  updater: null as unknown as EventEmitter & Record<string, unknown>,
  squirrel: null as unknown as EventEmitter,
  github: {
    checkForUpdates: null as unknown as ReturnType<typeof vi.fn>,
    downloadUpdate: null as unknown as ReturnType<typeof vi.fn>,
  },
}));

vi.mock('electron', async () => {
  const { EventEmitter: Emitter } = await import('events');
  h.squirrel = new Emitter();
  return {
    app: {
      getPath: () => h.userData,
      getVersion: () => '1.93.0',
      getAppPath: () => '/app',
      isPackaged: true,
      quit: vi.fn(),
    },
    autoUpdater: h.squirrel,
    BrowserWindow: {
      getAllWindows: () => [
        {
          webContents: {
            send: (_channel: string, payload: { event: string; data?: unknown }) =>
              h.sent.push(payload),
          },
        },
      ],
    },
    ipcMain: {
      handle: (channel: string, fn: Handler) => {
        h.handlers[channel] = fn;
      },
      emit: vi.fn(),
    },
    nativeImage: { createFromPath: () => ({ setTemplateImage: () => {} }) },
    Tray: class {},
    shell: { showItemInFolder: vi.fn() },
    dialog: {
      showMessageBox: async (options: { buttons: string[]; detail: string }) => {
        h.dialogCalls.push(options);
        return { response: h.dialogResponse };
      },
    },
    Menu: { buildFromTemplate: () => ({}) },
    Notification: class {
      on() {}
      show() {}
    },
    net: {},
    session: { fromPartition: () => ({}) },
  };
});

vi.mock('electron-updater', async () => {
  const { EventEmitter: Emitter } = await import('events');
  const updater = Object.assign(new Emitter(), {
    currentVersion: { version: '1.93.0' },
    autoDownload: true,
    autoInstallOnAppQuit: false,
    logger: null,
    updateConfigPath: '',
    channel: null,
    allowPrerelease: false,
    allowDowngrade: false,
    httpExecutor: { download: vi.fn() },
    setFeedURL: vi.fn(),
    getFeedURL: () => 'feed',
    checkForUpdates: vi.fn(),
    downloadUpdate: vi.fn(),
    quitAndInstall: vi.fn(),
  });
  h.updater = updater as unknown as EventEmitter & Record<string, unknown>;
  return { autoUpdater: updater };
});

vi.mock('./logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
}));
vi.mock('./githubUpdater', () => {
  h.github.checkForUpdates = vi.fn();
  h.github.downloadUpdate = vi.fn();
  return { githubUpdater: h.github };
});
vi.mock('./biorouterVersion', () => ({ biorouterVersion: () => '1.93.0' }));
vi.mock('./recentDirs', () => ({ loadRecentDirs: () => [] }));
vi.mock('./updateCheckSchedule', () => ({
  scheduleUpdateChecks: (run: (reason: string) => Promise<void>) => {
    h.scheduled = run;
    return () => {};
  },
}));

const realPlatform = process.platform;
function onPlatform(platform: typeof process.platform) {
  Object.defineProperty(process, 'platform', { value: platform, configurable: true });
}

async function loadUpdater(platform: typeof process.platform) {
  onPlatform(platform);
  const mod = await import('./autoUpdater');
  mod.registerUpdateIpcHandlers();
  mod.setupAutoUpdater();
  return mod;
}

const updater = () =>
  h.updater as unknown as EventEmitter & {
    checkForUpdates: ReturnType<typeof vi.fn>;
    downloadUpdate: ReturnType<typeof vi.fn>;
    quitAndInstall: ReturnType<typeof vi.fn>;
  };

/** What electron-updater does when a check finds `version`. */
function checkFinds(version: string) {
  updater().checkForUpdates.mockImplementation(async () => {
    updater().emit('checking-for-update');
    updater().emit('update-available', { version });
    return { updateInfo: { version } };
  });
}

/** A download that succeeds, as MacUpdater reports it. */
function downloadSucceeds() {
  updater().downloadUpdate.mockImplementation(async () => {
    const version = (
      h.sent.find((e) => e.event === 'update-available')?.data as { version: string }
    )?.version;
    updater().emit('update-downloaded', { version });
    return [];
  });
}

const events = () => h.sent.map((e) => e.event);
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  // A fresh autoUpdater.ts per test. The mocked modules survive resetModules,
  // so their fakes are emptied by hand: listeners from an earlier test's
  // module would otherwise answer this test's events too.
  vi.resetModules();
  if (h.updater) {
    h.updater.removeAllListeners();
    for (const name of ['checkForUpdates', 'downloadUpdate', 'quitAndInstall']) {
      (h.updater[name] as ReturnType<typeof vi.fn>).mockReset();
    }
    Object.assign(h.updater, {
      autoDownload: true,
      autoInstallOnAppQuit: false,
      httpExecutor: { download: vi.fn() },
    });
  }
  h.squirrel?.removeAllListeners();
  h.github.checkForUpdates?.mockReset();
  h.github.downloadUpdate?.mockReset();
  h.handlers = {};
  h.sent = [];
  h.scheduled = null;
  h.dialogCalls = [];
  h.dialogResponse = 2;
  h.userData = fs.mkdtempSync(path.join(os.tmpdir(), 'br-updater-flow-'));
});

afterEach(() => {
  onPlatform(realPlatform);
  fs.rmSync(h.userData, { recursive: true, force: true });
});

describe('autoUpdater on macOS', () => {
  it('turns off electron-updater’s own download and makes downloads resumable', async () => {
    await import('electron-updater');
    const original = (h.updater as unknown as { httpExecutor: { download: unknown } }).httpExecutor
      .download;
    await loadUpdater('darwin');
    expect(h.updater.autoDownload).toBe(false);
    expect(h.updater.autoInstallOnAppQuit).toBe(true);
    expect(
      (h.updater as unknown as { httpExecutor: { download: unknown } }).httpExecutor.download
    ).not.toBe(original);
  });

  it('installs through electron-updater even after a failed check chose the fallback', async () => {
    await loadUpdater('darwin');
    // A check that fails the way that used to make the fallback stick.
    h.github.checkForUpdates.mockResolvedValue({ updateAvailable: false, latestVersion: '1.93.0' });
    updater().emit('error', new Error('net::ERR_CONNECTION_REFUSED'));
    await flush();
    expect(h.github.checkForUpdates).toHaveBeenCalled();

    checkFinds('1.93.2');
    downloadSucceeds();
    await h.scheduled!('periodic');
    await flush();
    expect(events()).toContain('update-preparing');
    expect(h.sent[h.sent.length - 1].usingFallback).toBe(false);

    h.squirrel.emit('update-downloaded');
    expect(events()[events().length - 1]).toBe('update-downloaded');

    const result = await h.handlers['install-update']();
    expect(result).toEqual({ ok: true });
    expect(updater().quitAndInstall).toHaveBeenCalledTimes(1);
    expect(h.dialogCalls).toHaveLength(0);
  });

  it('says why when there is nothing to install, instead of doing nothing', async () => {
    await loadUpdater('darwin');
    const result = await h.handlers['install-update']();
    expect(result).toMatchObject({ ok: false });
    expect(updater().quitAndInstall).not.toHaveBeenCalled();
  });

  it('does not download or stage the same update again on the next check', async () => {
    await loadUpdater('darwin');
    checkFinds('1.93.2');
    downloadSucceeds();
    await h.scheduled!('startup');
    await flush();
    h.squirrel.emit('update-downloaded');

    await h.scheduled!('periodic');
    await flush();
    expect(updater().downloadUpdate).toHaveBeenCalledTimes(1);
    // Still ready: the re-check did not knock the window back to "downloading".
    const state = (await h.handlers['get-update-state']()) as { status: string };
    expect(state.status).toBe('downloaded');
  });

  it('reports a failed download once, as a download error, not as a reason to fall back', async () => {
    await loadUpdater('darwin');
    checkFinds('1.93.2');
    updater().downloadUpdate.mockImplementation(async () => {
      const error = new Error('net::ERR_CONNECTION_REFUSED');
      updater().emit('error', error); // electron-updater dispatches it, then rejects
      throw error;
    });
    await h.scheduled!('startup');
    await flush();
    await flush();

    expect(h.github.checkForUpdates).not.toHaveBeenCalled();
    const errors = h.sent.filter((e) => e.event === 'error');
    expect(errors).toHaveLength(1);
    expect(errors[0].data).toMatch(/net::ERR_CONNECTION_REFUSED.*try again/i);
  });

  it('turns a Squirrel.Mac failure while staging into install-failed', async () => {
    await loadUpdater('darwin');
    checkFinds('1.93.2');
    downloadSucceeds();
    await h.scheduled!('startup');
    await flush();
    updater().emit('error', new Error('Cannot update while running on a read-only volume.'));
    expect(h.sent[h.sent.length - 1]).toMatchObject({
      event: 'install-failed',
      data: { message: 'Cannot update while running on a read-only volume.' },
    });
  });
});

describe('autoUpdater assisted install on Windows', () => {
  it('classifies the downloaded installer, not the Downloads folder', async () => {
    await loadUpdater('win32');
    h.github.checkForUpdates.mockResolvedValue({
      updateAvailable: true,
      latestVersion: '1.93.2',
      downloadUrl: 'https://github.com/o/r/releases/download/v1.93.2/Biorouter-Setup-1.93.2.exe',
      releaseUrl: 'https://github.com/o/r/releases/tag/v1.93.2',
    });
    // A real file: the install checks it exists. `extractedPath` is the folder,
    // as githubUpdater returns it.
    const installer = path.join(h.userData, 'Biorouter-1.93.2.exe');
    fs.writeFileSync(installer, 'MZ');
    h.github.downloadUpdate.mockResolvedValue({
      success: true,
      downloadPath: installer,
      extractedPath: h.userData,
    });
    updater().checkForUpdates.mockRejectedValue(new Error('HttpError: 404'));

    await h.handlers['check-for-updates']();
    const downloaded = (await h.handlers['download-update']()) as { success: boolean };
    expect(downloaded.success).toBe(true);
    const installed = await h.handlers['install-update']();

    expect(installed).toEqual({ ok: true });
    expect(h.dialogCalls).toHaveLength(1);
    expect(h.dialogCalls[0].buttons).not.toContain('Open Folder Only');
    expect(h.dialogCalls[0].detail).toMatch(/installer/i);
  });
});
