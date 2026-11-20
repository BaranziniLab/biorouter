import { describe, expect, it } from 'vitest';
import { menuBarWindows, revealFromMenuBar, type MenuBarWindow } from './menuBarReveal';

/**
 * What a click on the menu bar icon does to one window. Whether macOS really brings the app in
 * front of another one is native behaviour no test here can see; these pin the calls that ask for
 * it, and the windows they are made on.
 */
function fakeWindow(state: { minimized?: boolean; visible?: boolean; destroyed?: boolean } = {}) {
  const calls: string[] = [];
  let minimized = state.minimized ?? false;
  let visible = state.visible ?? true;
  const win: MenuBarWindow & { calls: string[] } = {
    calls,
    isDestroyed: () => state.destroyed ?? false,
    isMinimized: () => minimized,
    isVisible: () => visible && !minimized,
    restore: () => {
      calls.push('restore');
      minimized = false;
    },
    show: () => {
      calls.push('show');
      visible = true;
    },
    focus: () => calls.push('focus'),
  };
  return win;
}

describe('revealFromMenuBar', () => {
  it('shows a window that is already visible on macOS, which activates Biorouter (#550)', () => {
    const win = fakeWindow();
    revealFromMenuBar(win, 'darwin');
    expect(win.calls).toEqual(['show', 'focus']);
  });

  it.each(['win32', 'linux'])('only focuses a visible window on %s', (platform) => {
    const win = fakeWindow();
    revealFromMenuBar(win, platform);
    expect(win.calls).toEqual(['focus']);
  });

  it.each(['darwin', 'win32', 'linux'])('shows a hidden window on %s', (platform) => {
    const win = fakeWindow({ visible: false });
    revealFromMenuBar(win, platform);
    expect(win.calls).toEqual(['show', 'focus']);
  });

  it('restores a minimized window first on macOS', () => {
    const win = fakeWindow({ minimized: true });
    revealFromMenuBar(win, 'darwin');
    expect(win.calls).toEqual(['restore', 'show', 'focus']);
  });

  it('restores a minimized window on Windows without showing it again', () => {
    const win = fakeWindow({ minimized: true });
    revealFromMenuBar(win, 'win32');
    expect(win.calls).toEqual(['restore', 'focus']);
  });
});

describe('menuBarWindows', () => {
  it('keeps chat windows in order and drops every other window', () => {
    const first = Object.assign(fakeWindow(), { id: 1 });
    const capture = Object.assign(fakeWindow({ visible: false }), { id: 2 });
    const second = Object.assign(fakeWindow({ minimized: true }), { id: 3 });
    const chat = new Set([1, 3]);
    expect(menuBarWindows([first, capture, second], (win) => chat.has(win.id))).toEqual([
      first,
      second,
    ]);
  });

  it('drops a chat window that is being destroyed', () => {
    const closing = Object.assign(fakeWindow({ destroyed: true }), { id: 1 });
    expect(menuBarWindows([closing], () => true)).toEqual([]);
  });
});
