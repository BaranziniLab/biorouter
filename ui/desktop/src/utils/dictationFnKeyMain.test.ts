import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { THEME_FAMILY_IDS } from '../styles/themes.generated';
import { indicatorPalette, indicatorThemeScript } from './dictationIndicator';
import { parseThemeAppearance, resolveAppearance } from './themeAppearance';

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
    // windowsHide is stated for the console-window census, which reads every
    // spawn site without knowing this helper is macOS-only.
    expect(source).toContain(
      "spawn(file, [], { stdio: ['pipe', 'pipe', 'ignore'], env: {}, windowsHide: true })"
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
  it('plans each intent with its gesture id from the latched route, and keeps the latch', () => {
    const handler = slice('function handleFnIntent(', 'function startFnAnywhere(');
    expect(handler).toContain('function handleFnIntent(intent: FnIntent, gesture: number): void');
    expect(handler).toContain('const plan = planFnIntent({ intent, gesture }, fnLatch, world);');
    expect(handler).toContain('fnLatch = plan.latch;');
    expect(handler).toContain(
      'for (const action of plan.actions) runFnAction(action, intent, world);'
    );
    // The old per-intent routing by focus is gone: one latch per gesture.
    expect(source).not.toContain('fnHold');
    expect(source).not.toMatch(/fnStartRoute\(|fnToggleRoute\(/);
    // The helper service hands the gesture id over.
    expect(slice('function startFnKeyShortcut(', 'function fnWorld(')).toContain(
      'onIntent: handleFnIntent,'
    );
  });

  it('into other apps only as the global shortcut does: public terms, governed by the switch', () => {
    const start = slice('function startFnAnywhere(', 'function runFnAction(');
    expect(start).toContain(
      'void startAnywhere(source, false, preview, (generation) => fnQuietGeneration === generation);'
    );
    const run = slice('function runFnAction(', 'function stopAnywhere(');
    expect(run).toContain("const source: AnywhereSource = preview ? 'preview' : 'shortcut';");
    expect(run).not.toContain("'tray'");
    expect(count(source, 'startFnAnywhere(')).toBe(3);
    expect(source).not.toMatch(/startAnywhere\('shortcut', true/);
    // Preview on fn's route carries public terms whatever is asked.
    expect(slice('async function startAnywhere(', 'function cancelAnywhere()')).toContain(
      "if (source === 'shortcut' || preview?.anyView) privateConfirmed = false;"
    );
    const world = slice('function fnWorld(', 'function sendFnIntent(');
    expect(world).toContain(
      'fnKeySettings.enabled && fnKeySettings.otherApps && anywhereSettings.enabled && !anywhereOffRequested'
    );
  });

  it('a focused app preview or browser tab in the window in front is its own route', () => {
    const world = slice('function fnWorld(', 'function sendFnIntent(');
    expect(world).toContain(
      'focusedEmbeddedViewId(focusedApp, electronWebContents.getFocusedWebContents())'
    );
    expect(world).toContain(
      'focusedEmbedded: focusedApp && embeddedView ? { windowId: focusedApp.id, viewId: embeddedView } : null'
    );
    const run = slice('function runFnAction(', 'function stopAnywhere(');
    expect(run).toContain('preview = { owner, viewId: action.viewId, anyView: true };');
    // Its focus check and insert admit a browser tab; the preview button's keep the managed ones.
    const start = slice('async function startAnywhere(', 'function cancelAnywhere()');
    expect(start).toContain(
      'const focused = preview.anyView ? await embeddedViewHasFocusedField(preview.owner, preview.viewId) : await managedAppHasFocusedField(preview.owner, preview.viewId);'
    );
    expect(
      slice('async function insertIntoAnywherePreview(', 'async function showAnywhereReview(')
    ).toContain(
      'const inserted = target.anyView ? await insertIntoEmbeddedFocusedField(target.owner, target.viewId, text) : await insertIntoManagedAppFocusedField(target.owner, target.viewId, text);'
    );
  });

  it('a release or another key reaches only what its own press started there', () => {
    const run = slice('function runFnAction(', 'function stopAnywhere(');
    const stop = run.slice(run.indexOf("case 'stop':"), run.indexOf("case 'cancel':"));
    expect(stop).toContain("if (intent !== 'toggle' && !fnAnywhereLive()) {");
    expect(stop).toContain('stopAnywhere();');
    const cancel = run.slice(run.indexOf("case 'cancel':"));
    expect(cancel).toContain('if (fnAnywhereLive()) cancelAnywhere();');
    expect(slice('function fnAnywhereLive()', 'function fnMayPrimeAnywhere()')).toContain(
      "fnAnywhereGeneration !== null && fnAnywhereGeneration === anywhereGeneration && (anywhereState.phase === 'arming' || anywhereState.phase === 'recording')"
    );
  });
});

describe('capture from fn down', () => {
  it('into other apps only when nothing would be asked first, and silent until the hold', () => {
    const may = slice('function fnMayPrimeAnywhere()', 'function handleFnIntent(');
    expect(may).toContain('anywhereBackendIdentity()?.managed === true');
    expect(may).toContain("systemPreferences.getMediaAccessStatus('microphone') === 'granted'");
    const run = slice('function runFnAction(', 'function stopAnywhere(');
    expect(run).toContain(
      "case 'prime': forget(); if (fnMayPrimeAnywhere()) startFnAnywhere(source, preview, true); return;"
    );
    // The hold, or the double-tap's second press, makes a refusal visible again.
    expect(run).toContain(
      "case 'start': // The hold is confirmed, or the double-tap keeps its second press"
    );
    expect(run).toContain('if (fnAnywhereLive()) { fnQuietGeneration = null; return; }');
    expect(run).toContain(
      'if (!fnDictationRunning(world)) startFnAnywhere(source, preview, false);'
    );
    const start = slice('async function startAnywhere(', 'function cancelAnywhere()');
    expect(start).toContain("availability === 'no-model' && !quiet?.(generation)");
  });

  it('a release while arming stops the dictation when it starts instead of dropping it', () => {
    const stop = slice('function stopAnywhere(): void', 'function indicatorWorkArea()');
    expect(stop).not.toContain('cancelAnywhere(');
    expect(stop).not.toContain('startAnywhere(');
    expect(stop).toContain('if (anywhereStartSentGeneration === anywhereGeneration) {');
    expect(stop).toContain('anywhereStopWhenStarted = anywhereGeneration;');
    const send = slice(
      'function sendAnywhereStartIfReady()',
      "ipcMain.on('dictation:anywhere:capture-ready'"
    );
    expect(send).toContain(
      "if (anywhereStopWhenStarted === generation && anywhereState.phase === 'arming') { anywhereStopWhenStarted = null; anywhereState = nextAnywhereState(anywhereState, { type: 'stop' });"
    );
    expect(send.indexOf("type: 'start',")).toBeLessThan(
      send.indexOf("if (anywhereState.phase === 'transcribing')")
    );
    // A cancel or a new start forgets a stop that belonged to an older generation.
    expect(slice('function cancelAnywhere()', 'function isDictationCaptureSender(')).toContain(
      'anywhereStopWhenStarted = null;'
    );
  });
});

describe('the fn trace (unpackaged only)', () => {
  it('logs helper lines and intents only when the seam is on, through the one formatter', () => {
    const service = slice('function startFnKeyShortcut(', 'function fnWorld(');
    expect(service).toContain(
      "onTrace: dictationSeams.fnTrace ? (line) => log.info(formatFnTrace({ kind: 'line', ...line })) : undefined,"
    );
    const handler = slice('function handleFnIntent(', 'function startFnAnywhere(');
    expect(handler).toContain(
      "if (dictationSeams.fnTrace) log.info(formatFnTrace({ kind: 'intent',"
    );
    expect(count(source, 'formatFnTrace(')).toBe(2);
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

describe('the theme the indicator and the review card wear (DD-99)', () => {
  const handler = slice("ipcMain.on('theme:appearance'", "ipcMain.handle('theme:appearance:get'");
  const announce = slice('function announceThemeAppearance()', "ipcMain.on('theme:appearance'");

  it('hears only a chat window’s main frame, and keeps only a report that parses whole', () => {
    // `isDictationSender` is the chat window check: `windowMap`, its own webContents, main frame.
    expect(handler).toMatch(
      /^ipcMain\.on\('theme:appearance', \(event, raw: unknown\) => \{ if \(!isDictationSender\(event\)\) return; const report = parseThemeAppearance\(raw\); if \(!report\) return;/
    );
    const sender = slice('function isDictationSender(', 'let anywhereCaptureWindow');
    expect(sender).toContain('windowMap.has(win.id)');
    expect(sender).toContain('event.senderFrame === event.sender.mainFrame');
    // The words are kept for the next launch, beside the canvas mode.
    expect(handler).toContain('settings.themeFamily = report.family;');
    expect(handler).toContain('settings.themeFollowsSystem = report.followsSystem;');
    expect(handler).toContain('saveSettings(settings);');
  });

  it('never hands a renderer’s string to the page: the palette is the generator’s, for words main checked', () => {
    for (const text of [handler, announce]) {
      expect(text).not.toContain('executeJavaScript');
      expect(text).not.toContain('raw.');
    }
    expect(announce).toContain(
      'dictationIndicator?.setPalette(indicatorPalette(view.family, view.mode));'
    );
    expect(announce).toContain('const view = themeAppearanceNow();');
    expect(slice('function themeAppearanceNow()', 'function announceThemeAppearance()')).toContain(
      'resolveAppearance(themeAppearanceReport, loadSettings(), nativeTheme.shouldUseDarkColors)'
    );
    const update = slice('function updateDictationIndicator()', "ipcMain.on('dictation:indicator'");
    expect(update).toContain(
      'palette: () => { const view = themeAppearanceNow(); return indicatorPalette(view.family, view.mode); },'
    );
    // Behaviour: a hostile report is dropped whole, so nothing of it reaches the theme script.
    for (const raw of [
      { family: "parchment'); alert(1); ('", mode: 'dark', followsSystem: false },
      { family: 'parchment', mode: '#000', followsSystem: false },
      { family: 'parchment', mode: 'dark', followsSystem: 'yes' },
      { family: 'constructor', mode: 'dark', followsSystem: false },
      'parchment',
      null,
    ]) {
      expect(parseThemeAppearance(raw), JSON.stringify(raw)).toBeNull();
    }
    // Every report that parses gives a script of 6-digit hex and the generator's shadow, only.
    for (const family of THEME_FAMILY_IDS) {
      for (const mode of ['light', 'dark'] as const) {
        const report = parseThemeAppearance({ family, mode, followsSystem: false });
        expect(report).not.toBeNull();
        const view = resolveAppearance(report, null, false);
        const script = indicatorThemeScript(indicatorPalette(view.family, view.mode));
        expect(script).not.toBeNull();
        const args = JSON.parse(
          `[${script!.slice(script!.indexOf('__brIndicatorTheme(') + 19, -1)}]`
        );
        expect(args[0]).toMatch(/^#[0-9a-f]{6}$/i);
        expect(args[1]).toMatch(/^#[0-9a-f]{6}$/i);
        expect(script).not.toContain(family);
      }
    }
  });

  it('follows the system while the person chose System, and reaches the card while it is open', () => {
    expect(source).toContain("nativeTheme.on('updated', announceThemeAppearance);");
    expect(announce).toContain('card.setBackgroundColor(WINDOW_CANVAS[view.mode]);');
    expect(announce).toContain("card.webContents.send('theme:appearance:changed', view);");
    const get = slice(
      "ipcMain.handle('theme:appearance:get'",
      "ipcMain.handle('dictation:anywhere:preview-focused'"
    );
    expect(get).toContain(
      'if (!isDictationSender(event) && !isDictationReviewSender(event)) return null; return themeAppearanceNow();'
    );
    // The card's window paints main's canvas before its page does.
    expect(
      slice('async function createAnywhereWindow(', 'async function insertIntoAnywherePreview(')
    ).toContain(
      '...(hidden ? {} : { backgroundColor: WINDOW_CANVAS[themeAppearanceNow().mode] }),'
    );
  });
});

describe('a page that stops reporting', () => {
  it('loses its report when it crashes or is replaced, so a reload never leaves a recording', () => {
    const reports = slice("ipcMain.on('dictation:indicator'", "ipcMain.handle('dictation:fn:get'");
    expect(reports).toContain(
      'const forget = () => { if (dictationReports.delete(id)) updateDictationIndicator(); };'
    );
    expect(reports).toContain("contents.on('render-process-gone', forget);");
    expect(reports).toContain(
      "contents.on('did-start-navigation', (details) => { if (details.isMainFrame && !details.isSameDocument) forget(); });"
    );
    expect(reports).toContain("contents.once('destroyed', () => {");
    // And the new document says idle before anything else (preload, the preload-bridge lane).
    const preload = fs.readFileSync(path.join(__dirname, '../preload.ts'), 'utf8');
    expect(preload).toMatch(/ipcRenderer\.send\('dictation:indicator', idleDictation\);/);
  });
});

describe('fn’s preview route in the embedded browser', () => {
  const browser = flat(fs.readFileSync(path.join(__dirname, 'embeddedBrowser.ts'), 'utf8'));
  const part = (from: string, to: string) => {
    const start = browser.indexOf(from);
    const end = browser.indexOf(to, start + 1);
    expect(start, from).toBeGreaterThan(0);
    return browser.slice(start, end < 0 ? undefined : end);
  };

  it('names only a shown view of the window in front whose page has keyboard focus', () => {
    const focused = part('export function focusedEmbeddedViewId(', 'function dictationEntry(');
    expect(focused).toContain('if (entry.window !== window || !entry.visible) continue;');
    expect(focused).toContain(
      'if (entry.view.webContents.isDestroyed() || entry.view.webContents !== focused) continue;'
    );
  });

  it('checks and inserts in the isolated world the page cannot reach, never while an agent types', () => {
    const entry = part(
      'function dictationEntry(',
      'export async function embeddedViewHasFocusedField('
    );
    expect(entry).toContain('if (entry.managed && !entry.managed.isActive()) return null;');
    expect(entry).toContain('if (entry.agentInputDepth > 0) return null;');
    const check = part(
      'export async function embeddedViewHasFocusedField(',
      'export async function insertIntoEmbeddedFocusedField('
    );
    expect(check).toContain(
      'executeJavaScriptInIsolatedWorld(DICTATION_FOCUS_WORLD_ID, [ { code: FOCUSED_EDITABLE_EXPRESSION }, ])'
    );
    const insert = part('export async function insertIntoEmbeddedFocusedField(', '\u0000');
    expect(insert).toContain(
      'if (!text || text.length > 32_000 || !(await embeddedViewHasFocusedField(window, viewId))) return false;'
    );
    expect(insert).toContain('entry.view.webContents.insertText(text);');
    // The preview button's own pair is unchanged: managed previews only.
    expect(
      part(
        'export async function managedAppHasFocusedField(',
        'export async function insertIntoManagedAppFocusedField('
      )
    ).toContain(
      'if (!entry?.managed?.isActive() || entry.view.webContents.isDestroyed()) return false;'
    );
  });
});
