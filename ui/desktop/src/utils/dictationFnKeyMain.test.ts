import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Electron main's wiring of the fn key shortcut and the dictation indicator. Main cannot be
 * loaded in jsdom, so these read its source; `fnGesture.test.ts`, `fnKeyHelper.test.ts`,
 * `dictationFnKey.test.ts` and `dictationIndicator.test.ts` cover the modules it calls.
 */
// Each run of white space as one space and none just inside a parenthesis, so the literals
// below pin what the code says and not where Prettier breaks its lines.
const flat = (text: string) => text.replace(/\s+/g, ' ').replace(/\( /g, '(').replace(/ \)/g, ')');
const source = flat(fs.readFileSync(path.join(__dirname, '../main.ts'), 'utf8'));

function slice(from: string, to: string): string {
  const start = source.indexOf(from);
  const end = source.indexOf(to, start + 1);
  expect(start, from).toBeGreaterThan(0);
  expect(end, to).toBeGreaterThan(start);
  return source.slice(start, end);
}

const count = (text: string, needle: string) => text.split(needle).length - 1;

describe('the fn key setting', () => {
  it('is written only by main, owner-only, by an atomic rename', () => {
    const write = slice('async function writeFnKeySettings(', 'function fnKeyView()');
    expect(write).toContain('fs.mkdir(path.dirname(filename), { recursive: true, mode: 0o700 })');
    expect(write).toContain('fs.writeFile(temporary, JSON.stringify(settings), { mode: 0o600 })');
    expect(write).toContain('fs.rename(temporary, filename)');
    expect(source).toContain("path.join(biorouterConfigDir(), 'dictation', 'fn-key.json')");
    const src = path.join(__dirname, '..');
    const writers: string[] = [];
    const walk = (dir: string) => {
      for (const name of fs.readdirSync(dir)) {
        const file = path.join(dir, name);
        if (fs.statSync(file).isDirectory()) walk(file);
        else if (/\.(ts|tsx)$/.test(name) && !/\.test\.tsx?$/.test(name)) {
          const text = fs.readFileSync(file, 'utf8');
          if (text.includes('fn-key.json') && /\bwriteFile(Sync)?\(/.test(text)) writers.push(name);
        }
      }
    };
    walk(src);
    expect(writers).toEqual(['main.ts']);
  });

  it('starts the helper with no environment, and stops it and the indicator at quit', () => {
    expect(source).toContain(
      "spawn: (file) => spawn(file, [], { stdio: ['pipe', 'pipe', 'ignore'], env: {} })"
    );
    const quit = slice("app.on('will-quit', async () => {", 'clearAnywhereReview();');
    expect(quit).toContain('fnKeyService?.dispose();');
    expect(quit).toContain('dictationIndicator?.dispose();');
  });

  it('asks macOS for Input Monitoring only from Settings’ button', () => {
    expect(count(source, 'requestPermission()')).toBe(1);
    const open = slice("ipcMain.handle('dictation:fn:open-settings'", 'function ');
    expect(open).toContain('fnKeyService?.requestPermission();');
    expect(open).toContain('Privacy_ListenEvent');
  });
});

describe('where fn dictates', () => {
  it('into other apps only as the global shortcut does: public terms, governed by the switch', () => {
    const handler = slice('function handleFnIntent(', 'function indicatorWorkArea()');
    expect(count(handler, "startAnywhere('shortcut', false)")).toBe(3);
    expect(handler).not.toContain("startAnywhere('tray'");
    expect(handler).not.toMatch(/startAnywhere\('shortcut', true\)/);
    const world = slice('function fnWorld(', 'function sendFnIntent(');
    expect(world).toContain(
      'fnKeySettings.enabled && fnKeySettings.otherApps && anywhereSettings.enabled && !anywhereOffRequested'
    );
  });
});

describe('the indicator window', () => {
  it('never takes focus, ignores the mouse, stays out of the Dock and Mission Control', () => {
    const create = slice('function createIndicatorWindow()', 'function updateDictationIndicator()');
    for (const option of [
      'focusable: false',
      'skipTaskbar: true',
      'hiddenInMissionControl: true',
      'transparent: true',
      'frame: false',
      "type: 'panel'",
      'sandbox: true',
      'contextIsolation: true',
      'nodeIntegration: false',
    ]) {
      expect(create, option).toContain(option);
    }
    expect(create).not.toContain('preload');
    expect(create).toContain('win.setIgnoreMouseEvents(true);');
    expect(create).toMatch(
      /win\.setVisibleOnAllWorkspaces\(true, \{ visibleOnFullScreen: true, skipTransformProcessType: true,? \}\);/
    );
    expect(create).toContain("win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));");
    expect(create).toContain(
      "win.webContents.on('will-navigate', (event) => event.preventDefault());"
    );
  });
});
