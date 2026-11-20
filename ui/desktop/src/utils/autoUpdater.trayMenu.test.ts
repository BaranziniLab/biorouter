// @vitest-environment node

/**
 * The tray menu's Home, New Chat, Settings, Extensions and Skills items bring Biorouter's chat
 * windows forward and send the view to one of them. Electron is a fake; the module under test is
 * the real `autoUpdater.ts`, which builds that menu.
 *
 * Each case is a bug the items shipped with:
 *  - on macOS a chat window that was already visible stayed behind the app in front, because only
 *    `focus()` was called on it (the tray click had the same bug, #550);
 *  - the hidden dictation capture window and a dismissed dictation review card were shown as
 *    well, and the newest of them, not a chat window, was sent the view;
 *  - with only those windows left, the menu showed them instead of opening a chat.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MenuItemConstructorOptions, Tray } from 'electron';

const h = vi.hoisted(() => ({
  windows: [] as FakeWindow[],
  template: [] as MenuItemConstructorOptions[],
  emitted: [] as unknown[][],
}));

type FakeWindow = {
  id: number;
  calls: string[];
  sent: unknown[][];
  minimized: boolean;
  visible: boolean;
  destroyed: boolean;
  isDestroyed(): boolean;
  isMinimized(): boolean;
  isVisible(): boolean;
  restore(): void;
  show(): void;
  focus(): void;
  webContents: { send(...args: unknown[]): void };
};

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
    BrowserWindow: { getAllWindows: () => h.windows },
    ipcMain: {
      handle: vi.fn(),
      emit: (...args: unknown[]) => {
        h.emitted.push(args);
        return true;
      },
    },
    nativeImage: { createFromPath: () => ({ setTemplateImage: () => {} }) },
    Tray: class {},
    shell: { showItemInFolder: vi.fn() },
    dialog: { showMessageBox: vi.fn() },
    Menu: {
      buildFromTemplate: (template: MenuItemConstructorOptions[]) => {
        h.template = template;
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

function fakeWindow(id: number, state: Partial<Pick<FakeWindow, 'minimized' | 'visible'>> = {}) {
  const win: FakeWindow = {
    id,
    calls: [],
    sent: [],
    minimized: state.minimized ?? false,
    visible: state.visible ?? true,
    destroyed: false,
    isDestroyed: () => win.destroyed,
    isMinimized: () => win.minimized,
    isVisible: () => win.visible && !win.minimized,
    restore: () => {
      win.calls.push('restore');
      win.minimized = false;
    },
    show: () => {
      win.calls.push('show');
      win.visible = true;
    },
    focus: () => win.calls.push('focus'),
    webContents: { send: (...args: unknown[]) => win.sent.push(args) },
  };
  return win;
}

const realPlatform = process.platform;
function onPlatform(platform: typeof process.platform) {
  Object.defineProperty(process, 'platform', { value: platform, configurable: true });
}

const chatIds = new Set<number>();
const fakeTray = {
  setImage: vi.fn(),
  setToolTip: vi.fn(),
  setContextMenu: vi.fn(),
  popUpContextMenu: vi.fn(),
} as unknown as Tray;

async function clickTrayItem(platform: typeof process.platform, label: string) {
  onPlatform(platform);
  const mod = await import('./autoUpdater');
  mod.setTrayRef(fakeTray, (win) => chatIds.has(win.id));
  const item = h.template.find((entry) => entry.label === label);
  expect(item, `no tray item ${label}`).toBeDefined();
  (item!.click as () => void)();
}

beforeEach(() => {
  vi.resetModules();
  h.windows = [];
  h.template = [];
  h.emitted = [];
  chatIds.clear();
});

afterEach(() => {
  onPlatform(realPlatform);
});

describe('the tray menu brings chat windows forward', () => {
  it('shows a visible chat window on macOS, so it comes in front of the active app', async () => {
    const chat = fakeWindow(1);
    chatIds.add(1);
    h.windows = [chat];
    await clickTrayItem('darwin', 'Settings');
    expect(chat.calls).toEqual(['show', 'focus']);
    expect(chat.sent).toEqual([['set-view', 'settings']]);
  });

  it('only focuses a visible chat window on Windows, as before', async () => {
    const chat = fakeWindow(1);
    chatIds.add(1);
    h.windows = [chat];
    await clickTrayItem('win32', 'Extensions');
    expect(chat.calls).toEqual(['focus']);
    expect(chat.sent).toEqual([['set-view', 'extensions']]);
  });

  it('restores a minimized chat window before showing it', async () => {
    const chat = fakeWindow(1, { minimized: true });
    chatIds.add(1);
    h.windows = [chat];
    await clickTrayItem('darwin', 'Skills');
    expect(chat.calls).toEqual(['restore', 'show', 'focus']);
  });

  it('leaves the dictation windows hidden and sends the view to a chat window', async () => {
    const chat = fakeWindow(1);
    const capture = fakeWindow(2, { visible: false });
    const reviewCard = fakeWindow(3, { visible: false });
    chatIds.add(1);
    h.windows = [chat, capture, reviewCard];
    for (const platform of ['darwin', 'win32'] as const) {
      await clickTrayItem(platform, 'Settings');
    }
    expect(capture.calls).toEqual([]);
    expect(reviewCard.calls).toEqual([]);
    expect(capture.sent).toEqual([]);
    expect(reviewCard.sent).toEqual([]);
    expect(chat.sent).toEqual([
      ['set-view', 'settings'],
      ['set-view', 'settings'],
    ]);
  });

  it('opens a chat window when only the dictation windows are left', async () => {
    const reviewCard = fakeWindow(3, { visible: false });
    h.windows = [reviewCard];
    await clickTrayItem('darwin', 'Home');
    expect(reviewCard.calls).toEqual([]);
    expect(h.emitted).toEqual([['create-chat-window', {}, undefined, '/work']]);
  });

  it('Check for Updates opens Settings in a chat window, never the hidden capture window', async () => {
    const capture = fakeWindow(2, { visible: false });
    const chat = fakeWindow(1, { minimized: true });
    chatIds.add(1);
    h.windows = [capture, chat];
    await clickTrayItem('darwin', 'Check for Updates');
    expect(capture.calls).toEqual([]);
    expect(capture.sent).toEqual([]);
    expect(chat.calls).toEqual(['restore', 'show', 'focus']);
    expect(chat.sent).toEqual([['set-view', 'settings', 'update']]);
  });

  it('Check for Updates opens a chat window on Settings when only dictation windows are left', async () => {
    const capture = fakeWindow(2, { visible: false });
    h.windows = [capture];
    await clickTrayItem('darwin', 'Check for Updates');
    expect(capture.calls).toEqual([]);
    expect(h.emitted).toEqual([
      ['create-chat-window', {}, undefined, '/work', undefined, undefined, 'settings'],
    ]);
  });

  it('the app menu’s Check for Updates uses the predicate main registered, with no tray', async () => {
    onPlatform('win32');
    const mod = await import('./autoUpdater');
    const capture = fakeWindow(2, { visible: false });
    const chat = fakeWindow(1);
    h.windows = [capture, chat];
    mod.openUpdateSettings();
    // Nothing registered: no window counts as a chat window, so none is shown.
    expect(capture.calls).toEqual([]);
    expect(chat.calls).toEqual([]);
    h.emitted = [];
    mod.setChatWindowPredicate((win) => win.id === 1);
    mod.openUpdateSettings();
    expect(capture.calls).toEqual([]);
    expect(chat.calls).toEqual(['focus']);
    expect(chat.sent).toEqual([['set-view', 'settings', 'update']]);
  });

  it('skips a chat window that is being destroyed', async () => {
    const closing = fakeWindow(1);
    closing.destroyed = true;
    const chat = fakeWindow(2);
    chatIds.add(1).add(2);
    h.windows = [closing, chat];
    await clickTrayItem('darwin', 'New Chat');
    expect(closing.calls).toEqual([]);
    expect(chat.sent).toEqual([['set-view', '']]);
  });
});
