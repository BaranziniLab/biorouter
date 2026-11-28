// @vitest-environment node

/**
 * **Copy last dictation** in the tray menu (F10, F15, DD-102). Main holds the last text Biorouter
 * typed into another app, or a review card's that a new start closed, for what is left of five
 * minutes; the tray offers the item only while it holds one, and the item's click is the one way
 * that text reaches the clipboard (`copyLastDictation` in `main.ts`). Electron is a fake; the
 * module under test is the real `autoUpdater.ts`, which builds the menu.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { MenuItemConstructorOptions, Tray } from 'electron';
import { DICTATION_ACTIONS } from '../components/dictation/copy';

const h = vi.hoisted(() => ({
  template: [] as MenuItemConstructorOptions[],
  builds: 0,
}));

vi.mock('electron', async () => {
  const { EventEmitter: Emitter } = await import('events');
  return {
    app: {
      getPath: () => '/tmp',
      getVersion: () => '1.94.6',
      getAppPath: () => '/app',
      isPackaged: false,
      quit: vi.fn(),
    },
    autoUpdater: new Emitter(),
    BrowserWindow: { getAllWindows: () => [] },
    ipcMain: { handle: vi.fn(), emit: vi.fn(() => true) },
    nativeImage: { createFromPath: () => ({ setTemplateImage: () => {} }) },
    Tray: class {},
    shell: { showItemInFolder: vi.fn() },
    dialog: { showMessageBox: vi.fn() },
    Menu: {
      buildFromTemplate: (template: MenuItemConstructorOptions[]) => {
        h.template = template;
        h.builds += 1;
        return {};
      },
    },
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
  return { autoUpdater: Object.assign(new Emitter(), { currentVersion: { version: '1.94.6' } }) };
});
vi.mock('./logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
}));
vi.mock('./githubUpdater', () => ({ githubUpdater: {} }));
vi.mock('./biorouterVersion', () => ({ biorouterVersion: () => '1.94.6' }));
vi.mock('./recentDirs', () => ({ loadRecentDirs: () => ['/work'] }));

const fakeTray = {
  setImage: vi.fn(),
  setToolTip: vi.fn(),
  setContextMenu: vi.fn(),
  popUpContextMenu: vi.fn(),
} as unknown as Tray;

const LABEL = DICTATION_ACTIONS.copyLastDictation;

/** The tray's dictation actions as main passes them, with what main holds now. */
function trayActions(now: { phase: string; lastDictation: boolean }) {
  return {
    state: () => ({
      enabled: true,
      phase: now.phase,
      privateAvailable: false,
      lastDictation: now.lastDictation,
    }),
    start: vi.fn(),
    privateStart: vi.fn(),
    openSettings: vi.fn(),
    stop: vi.fn(),
    cancel: vi.fn(),
    copyLast: vi.fn(),
  };
}

async function trayModule() {
  const mod = await import('./autoUpdater');
  mod.setTrayRef(fakeTray, () => true);
  return mod;
}

const labels = () => h.template.map((item) => item.label ?? item.type);
const item = (label: string) => h.template.find((entry) => entry.label === label);

beforeEach(() => {
  vi.resetModules();
  h.template = [];
  h.builds = 0;
});

describe('the tray offers Copy last dictation', () => {
  it('is the copy catalog’s label', () => {
    expect(LABEL).toBe('Copy last dictation');
  });

  it('is absent while main holds no text', async () => {
    const mod = await trayModule();
    mod.setDictationAnywhereTrayActions(trayActions({ phase: 'idle', lastDictation: false }));
    expect(item(LABEL)).toBeUndefined();
    expect(item('Dictate to clipboard')).toBeDefined();
  });

  it('is offered while main holds a text, and its click is copyLast alone', async () => {
    const mod = await trayModule();
    const actions = trayActions({ phase: 'idle', lastDictation: true });
    mod.setDictationAnywhereTrayActions(actions);
    const copy = item(LABEL);
    expect(copy, `no tray item ${LABEL}`).toBeDefined();
    expect(copy!.enabled).not.toBe(false);
    (copy!.click as () => void)();
    expect(actions.copyLast).toHaveBeenCalledTimes(1);
    for (const other of [
      actions.start,
      actions.privateStart,
      actions.openSettings,
      actions.stop,
      actions.cancel,
    ])
      expect(other).not.toHaveBeenCalled();
    // Among the dictation items, before the separator that ends them.
    const order = labels();
    expect(order.indexOf(LABEL)).toBeGreaterThan(order.indexOf('Include my private terms'));
    expect(order[order.indexOf(LABEL) + 1]).toBe('separator');
  });

  it('is offered while the next dictation records, after a new start closed the review card', async () => {
    // F15: the closed card's text moved to the holder as the new dictation began.
    const mod = await trayModule();
    const actions = trayActions({ phase: 'recording', lastDictation: true });
    mod.setDictationAnywhereTrayActions(actions);
    const order = labels();
    expect(order.slice(0, 4)).toEqual(['Stop and copy', 'Cancel', LABEL, 'separator']);
    (item(LABEL)!.click as () => void)();
    expect(actions.copyLast).toHaveBeenCalledTimes(1);
    expect(actions.stop).not.toHaveBeenCalled();
  });

  it('comes and goes as main rebuilds the menu when it holds a text and when that text goes', async () => {
    const mod = await trayModule();
    const now = { phase: 'idle', lastDictation: false };
    mod.setDictationAnywhereTrayActions(trayActions(now));
    expect(item(LABEL)).toBeUndefined();
    // `holdLastDictation` in main: the text is held, the menu rebuilt.
    now.lastDictation = true;
    mod.updateTrayMenu(false);
    expect(item(LABEL)).toBeDefined();
    // `clearLastDictation` at expiry, quit or Off: the text goes, the menu rebuilt.
    now.lastDictation = false;
    mod.updateTrayMenu(false);
    expect(item(LABEL)).toBeUndefined();
  });
});
