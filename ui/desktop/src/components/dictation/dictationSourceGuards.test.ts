import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { PCM_CAPTURE_PROCESSOR } from './capture';

/**
 * Source guards for desktop capture (DD-64, DD-65). Each one pins a fact that a
 * unit test cannot see, because jsdom runs no Vite build, no content security
 * policy and no Chromium switches.
 */
const desktop = resolve(__dirname, '../../..');
const src = join(desktop, 'src');
const here = join(src, 'components/dictation');

const read = (path: string) => readFileSync(path, 'utf8');

/** Every text file under `root`, skipping dependencies and build output. */
function walk(root: string, out: string[] = []): string[] {
  const skip = new Set([
    'node_modules',
    'out',
    'dist',
    '.vite',
    'coverage',
    'test-results',
    'playwright-report',
    'preview-panel-evidence',
    'dictation-capture-evidence',
    'pdfjs',
    'bin',
    'web',
  ]);
  for (const name of readdirSync(root)) {
    if (skip.has(name) || name.startsWith('.git')) continue;
    const path = join(root, name);
    const stat = statSync(path);
    if (stat.isDirectory()) walk(path, out);
    else if (/\.(ts|tsx|js|mjs|cjs|mts|cts|json|html|md|yml|yaml|sh|css)$/.test(name))
      out.push(path);
  }
  return out;
}

describe('the AudioWorklet stays a real file', () => {
  const capture = read(join(here, 'capture.ts'));

  it('is imported only as ?url&no-inline', () => {
    const imports = capture.match(/from '[^']*pcmCapture\.worklet\.js[^']*'/g) ?? [];
    expect(imports).toEqual(["from './pcmCapture.worklet.js?url&no-inline'"]);
    for (const file of walk(src)) {
      if (file === join(here, 'capture.ts') || file.endsWith('.test.ts')) continue;
      expect(read(file), relative(desktop, file)).not.toMatch(/pcmCapture\.worklet\.js/);
    }
  });

  it('is never built as a blob: or data: module', () => {
    for (const name of ['capture.ts', 'dictationSession.ts', 'transport.ts']) {
      const text = read(join(here, name));
      expect(text, name).not.toMatch(/createObjectURL/);
      expect(text, name).not.toMatch(/data:text\/javascript/);
    }
  });

  it('is under Vite’s 4,096 B inline limit, so the no-inline query is what keeps it a file', () => {
    expect(statSync(join(here, 'pcmCapture.worklet.js')).size).toBeLessThan(4096);
  });

  it('registers the processor capture.ts creates', () => {
    expect(read(join(here, 'pcmCapture.worklet.js'))).toContain(
      `registerProcessor('${PCM_CAPTURE_PROCESSOR}'`
    );
  });

  it('is refused inlining by the renderer’s Vite config as well', () => {
    const config = read(join(desktop, 'vite.renderer.config.mts'));
    expect(config).toMatch(/assetsInlineLimit:\s*\(filePath: string\)\s*=>/);
    expect(config).toMatch(/\\\.worklet\\\.js/);
    expect(config).toMatch(/\?\s*false\s*:\s*undefined/);
  });
});

describe('test switches never reach a packaged app', () => {
  // Spelled in parts so this file does not trip its own scan.
  const fakeUi = ['use', 'fake', 'ui', 'for', 'media', 'stream'].join('-');

  /**
   * The 1-based lines of `text` that use the switch. A comment may name it to
   * say it is not used (PR-03's serve spec does); anything else on a line that
   * names it is a use.
   */
  const usesOf = (text: string): number[] =>
    text.split('\n').flatMap((line, index) => {
      const at = line.indexOf(fakeUi);
      if (at < 0) return [];
      const comment = line.slice(0, at).includes('//') || /^\s*(\*|\/\*|#)/.test(line);
      return comment ? [] : [index + 1];
    });

  it(`no file under ui/desktop uses the switch that bypasses the permission handler`, () => {
    const hits = walk(desktop).flatMap((file) =>
      usesOf(read(file)).map((line) => `${relative(desktop, file)}:${line}`)
    );
    expect(hits).toEqual([]);
  });

  it('the scan above sees a use when there is one, and lets a comment name it', () => {
    expect(usesOf(`app.commandLine.appendSwitch('${fakeUi}');`)).toEqual([1]);
    expect(usesOf(`const args = [\n  '--${fakeUi}',\n];`)).toEqual([2]);
    expect(usesOf(`// never ${fakeUi}: it bypasses the handler`)).toEqual([]);
    expect(usesOf(` * not ${fakeUi}`)).toEqual([]);
    expect(usesOf(`# ${fakeUi} is not used`)).toEqual([]);
  });

  it('the fake device switches are written in one module, which main.ts calls', () => {
    const fakeDevice = 'use-fake-device-for-media-stream';
    const writers = walk(src)
      .filter((file) => !file.endsWith('.test.ts'))
      .filter((file) => read(file).includes(fakeDevice))
      .map((file) => relative(src, file));
    expect(writers).toEqual(['utils/micPermissionSettings.ts']);
    const main = read(join(src, 'main.ts'));
    expect(main).toContain('fakeMicrophoneSwitches({');
    expect(main).toMatch(/isPackaged:\s*app\.isPackaged/);
    // At the top level of the module, so the switches are in place before
    // `ready`; inside `appMain` they would come too late for the media stack.
    expect(main).toMatch(/^for \(const \w+ of fakeMicrophoneSwitches\(\{$/m);
    expect(main.indexOf('fakeMicrophoneSwitches({')).toBeLessThan(
      main.indexOf('async function appMain(')
    );
  });
});

describe('the main process answers dictation IPC only from a Biorouter window', () => {
  // `main.ts` cannot be imported under vitest, so its text is what there is to
  // pin, as `main.openPath.test.ts` does for its handlers.
  const main = read(join(src, 'main.ts'));

  it('every renderer to main dictation channel returns early for any other sender', () => {
    const registrations = [...main.matchAll(/ipcMain\.(?:handle|on)\('(dictation:[^']+)'/g)];
    expect(registrations.map((match) => match[1]).sort()).toEqual([
      'dictation:busy',
      'dictation:mic-request',
      'dictation:mic-status',
      'dictation:open-mic-settings',
      'dictation:show-models-folder',
    ]);
    for (const match of registrations) {
      const body = main.slice(match.index!, main.indexOf('\n});', match.index!));
      const firstStatement = body.split('\n')[1]?.trim() ?? '';
      expect(firstStatement, match[1]).toMatch(/^if \(!isDictationSender\(event\)\) return\b/);
    }
  });

  it('a sender is the main frame of a window Biorouter opened', () => {
    const start = main.indexOf('function isDictationSender(');
    expect(start).toBeGreaterThan(-1);
    const body = main.slice(start, main.indexOf('\n}\n', start));
    expect(body).toContain('windowMap.has(win.id)');
    expect(body).toContain('win.webContents === event.sender');
    expect(body).toContain('event.senderFrame === event.sender.mainFrame');
  });

  it('opens the speech model folder only for this app’s own daemon, from main’s own rule', () => {
    const start = main.indexOf("ipcMain.handle('dictation:show-models-folder'");
    expect(start).toBeGreaterThan(-1);
    const body = main.slice(start, main.indexOf('\n});', start));
    expect(body).toContain('localBackendWindows.has(win.id)');
    expect(
      body.indexOf('localBackendWindows.has(win.id)'),
      'the window check must come before anything touches the file system'
    ).toBeLessThan(body.indexOf('showDictationModelsFolder('));
    // The folder is derived from this process's own environment; the renderer's string is only
    // the hint `utils/dictationModelsFolder.test.ts` holds to it.
    expect(body).toMatch(
      /showDictationModelsFolder\(\s*folder,\s*\{ platform: process\.platform, env: process\.env, homedir: os\.homedir\(\) \}/
    );
    expect(body).toContain('isPlainDirectory,');
    expect(body).toMatch(/shell\.openPath\(target\)/);
    // No second admission rule in main.ts to drift from the tested one.
    expect(main).not.toMatch(/function dictationModelsFolder\(/);
    expect(body).not.toMatch(/fs\.mkdir\(folder/);
    expect(main).toMatch(/if \(biorouterdResult\.managed\) localBackendWindows\.add\(windowId\);/);
    expect(main).toMatch(/localBackendWindows\.delete\(windowId\);/);
  });

  it('the busy record lives outside main.ts, so the updater can import it without a cycle', () => {
    expect(main).not.toMatch(/export function isDictationBusy/);
    expect(read(join(src, 'utils/dictationBusy.ts'))).toMatch(/export function isDictationBusy\(/);
  });

  it('a new document tells main that its window dictates nothing yet', () => {
    expect(read(join(src, 'preload.ts'))).toMatch(
      /^ipcRenderer\.send\('dictation:busy', false\);$/m
    );
  });
});

describe('one module talks to the daemon’s dictation routes', () => {
  it('only dictationApi.ts spells a /dictation/ route, so the generated client replaces one file', () => {
    const spellers = walk(src)
      .filter((file) => !/\.test\.tsx?$/.test(file) && !file.includes(`${join(src, 'api')}`))
      .filter((file) => /['`]\/dictation\//.test(read(file)))
      .map((file) => relative(src, file));
    expect(spellers).toEqual(['components/dictation/dictationApi.ts']);
  });
});
