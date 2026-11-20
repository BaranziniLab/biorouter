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

const isChatWindow = '(win) => windowMap.has(win.id)';

describe('the menu bar icon brings chat windows forward', () => {
  const showWindow = between('const showWindow = async () => {', '\n};\n');

  it('reveals only chat windows, never the dictation capture window or review card', () => {
    expect(showWindow).toContain(`menuBarWindows(BrowserWindow.getAllWindows(), ${isChatWindow})`);
  });

  it('reveals each one through the shared rule, which shows it on macOS (#550)', () => {
    expect(showWindow).toContain('revealFromMenuBar(win, process.platform)');
    expect(showWindow).not.toMatch(/\.(show|focus)\(\)/);
  });

  it('gives the tray menu the same chat windows', () => {
    const createTray = between('const createTray = () => {', '\n};\n');
    expect(createTray).toContain(`setTrayRef(tray, ${isChatWindow})`);
  });
});
