import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/**
 * A click on the menu bar icon (macOS and Windows) brings Biorouter's chat windows forward, and
 * the tray menu's items use the same rule. The rule is `utils/menuBarReveal.ts`, tested on its
 * own and through the real tray menu in `utils/autoUpdater.trayMenu.test.ts`; what is pinned here
 * is that `main.ts` uses it. Asserted against the source, as `main.openPath.test.ts` is: nothing
 * can import `main.ts` under vitest.
 */
const main = readFileSync(join(__dirname, 'main.ts'), 'utf8');

function between(start: string, end: string): string {
  const from = main.indexOf(start);
  expect(from, `no ${start}`).toBeGreaterThan(-1);
  const to = main.indexOf(end, from);
  expect(to, `no ${end} after ${start}`).toBeGreaterThan(from);
  return main.slice(from, to);
}

const chatWindows = 'menuBarWindows(BrowserWindow.getAllWindows(), isChatWindow)';

describe('one test for a chat window', () => {
  it('is the window map, registered with the updater before anything can reveal a window', () => {
    expect(main).toContain(
      'const isChatWindow = (win: BrowserWindow): boolean => windowMap.has(win.id);'
    );
    const defined = main.indexOf('const isChatWindow =');
    expect(main.indexOf('setChatWindowPredicate(isChatWindow);')).toBeGreaterThan(defined);
    // Every other path that counts or reveals windows asks it, never `getAllWindows().length`.
    expect(main).not.toMatch(/getAllWindows\(\)\.length === 0/);
  });
});

describe('the menu bar icon brings chat windows forward', () => {
  const showWindow = between('const showWindow = async () => {', '\n};\n');

  it('reveals only chat windows, never the dictation capture window or review card', () => {
    expect(showWindow).toContain(chatWindows);
  });

  it('reveals each one through the shared rule, which shows it on macOS (#550)', () => {
    expect(showWindow).toContain('revealFromMenuBar(win, process.platform)');
    expect(showWindow).not.toMatch(/\.(show|focus)\(\)/);
  });

  // Each click moved window N by 30 × N px from where it already was, so with two windows open
  // every click walked the second one further toward the bottom-right corner.
  it('never moves a window', () => {
    expect(showWindow).not.toMatch(/setBounds|setPosition/);
  });

  it('gives the tray menu the same chat windows', () => {
    const createTray = between('const createTray = () => {', '\n};\n');
    expect(createTray).toContain('setTrayRef(tray, isChatWindow)');
  });
});

describe('the other ways Biorouter comes forward count only chat windows', () => {
  it('Focus Biorouter Window reveals chat windows and opens one when there is none', () => {
    const focus = between('const focusWindow = () => {', '\n};\n');
    expect(focus).toContain(chatWindows);
    expect(focus).toContain('revealFromMenuBar(win, process.platform)');
    expect(focus).toContain('createNewWindow(app)');
    expect(focus).not.toMatch(/getAllWindows\(\);/);
  });

  it('a deep link or an extension file goes to a chat window, never a dictation window', () => {
    for (const [start, end] of [
      ['// Only focus existing windows for non-bot/workflow URLs', 'mainWindow.focus();'],
      ['async function ensureWindowAfterDeepLink(', '\n}\n'],
      ['// For other URL types, reuse an existing chat window', 'firstOpenWindow.focus();'],
      ['Stored pending deep link for processing after React ready', 'firstOpenWindow.focus();'],
      ['function handleBrxtFileOpen(', "win.webContents.send('open-brxt-file'"],
    ]) {
      expect(between(start, end), start).toContain(chatWindows);
    }
  });

  it('the Dock icon opens a chat window while only dictation windows are alive', () => {
    const activate = between("app.on('activate', () => {", '});');
    expect(activate).toContain(`${chatWindows}.length === 0`);
    expect(activate).toContain('createNewWindow(app)');
  });
});
