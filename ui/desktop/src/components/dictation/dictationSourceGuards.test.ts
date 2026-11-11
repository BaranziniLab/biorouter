import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { runInNewContext } from 'node:vm';
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

  it('does nothing when it runs outside a worklet, as esbuild bundles it', () => {
    // esbuild drops the `?url` query and bundles the file as code, so a browser test's bundle
    // that reaches a microphone (the transcript tests, through the message editor) runs it in
    // the page, where there is no AudioWorkletProcessor to extend.
    const source = read(join(here, 'pcmCapture.worklet.js'));
    expect(() => runInNewContext(source, {})).not.toThrow();

    class AudioWorkletProcessor {
      port = { onmessage: null, postMessage() {} };
    }
    const registered: Array<[string, unknown]> = [];
    runInNewContext(source, {
      AudioWorkletProcessor,
      registerProcessor: (name: string, processor: unknown) => registered.push([name, processor]),
    });
    expect(registered.map(([name]) => name)).toEqual([PCM_CAPTURE_PROCESSOR]);
    expect(Object.getPrototypeOf(registered[0][1])).toBe(AudioWorkletProcessor);
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

/** `text` with each run of white space as one space and none just inside a parenthesis. */
const flat = (text: string): string =>
  text.replace(/\s+/g, ' ').replace(/\( /g, '(').replace(/ \)/g, ')').trim();

/** Every `ipcMain` registration whose channel is a string, in any quoting. */
const IPC_REGISTRATION =
  /ipcMain\.(handle|handleOnce|on|once|addListener)\(\s*(['"`])((?:(?!\2)[^\\])*)\2/g;

/**
 * What a handler registered at `match` does before anything else: its callback's parameter list
 * and its first statement, both flattened, or null when the callback is not an arrow with a body.
 */
function handlerOpening(text: string, match: RegExpMatchArray) {
  const afterChannel = match.index! + match[0].length;
  const arrow = text.indexOf('=> {', afterChannel);
  if (arrow < 0) return null;
  const end = text.indexOf(';', arrow);
  return {
    parameters: flat(text.slice(afterChannel, arrow)),
    first: flat(text.slice(arrow + '=> {'.length, end < 0 ? undefined : end)),
  };
}

/**
 * The windows a renderer to main dictation channel may hear from, by the check that admits each:
 * a window in `windowMap`, or one of the two windows `createAnywhereWindow` opens for Dictate to
 * clipboard and Dictate into this app (S15, S18): the hidden capture window and the review card.
 */
const SENDER_CHECKS = {
  app: '!isDictationSender(event)',
  capture: '!isDictationCaptureSender(event)',
  review: '!isDictationReviewSender(event)',
} as const;

const DICTATION_CHANNELS: Record<string, Array<keyof typeof SENDER_CHECKS>> = {
  'dictation:anywhere:capture-ready': ['capture'],
  'dictation:anywhere:capture-reply': ['capture'],
  'dictation:anywhere:get': ['app'],
  'dictation:anywhere:preview-focused': ['app'],
  'dictation:anywhere:preview-start': ['app'],
  'dictation:anywhere:private-choice': ['review'],
  'dictation:anywhere:review-copy': ['review'],
  'dictation:anywhere:review-get': ['review'],
  'dictation:anywhere:set': ['app'],
  'dictation:anywhere:settings-ready': ['app'],
  'dictation:busy': ['app'],
  // The hidden capture window asks for the microphone as a composer does.
  'dictation:mic-request': ['app', 'capture'],
  'dictation:mic-status': ['app', 'capture'],
  'dictation:open-mic-settings': ['app'],
  'dictation:show-models-folder': ['app'],
};

/** The exact first statement a handler for `channel` must open with. */
function senderGuard(senders: Array<keyof typeof SENDER_CHECKS>): RegExp {
  const condition = senders
    .map((sender) => SENDER_CHECKS[sender].replace(/[()]/g, '\\$&'))
    .join(' && ');
  // The early return gives back a literal and nothing else, so it runs no code of its own.
  return new RegExp(`^if \\(${condition}\\) return(?: (?:null|false|true|'[a-z-]*'))?$`);
}

describe('the main process answers dictation IPC only from a Biorouter window', () => {
  // `main.ts` cannot be imported under vitest, so its text is what there is to
  // pin, as `main.openPath.test.ts` does for its handlers.
  const main = read(join(src, 'main.ts'));

  /** The text of the top level function `name`, up to its closing brace. */
  function functionText(name: string): string {
    const start = main.search(new RegExp(`^(?:async )?function ${name}\\(`, 'm'));
    expect(start, name).toBeGreaterThan(-1);
    return main.slice(start, main.indexOf('\n}\n', start));
  }

  it('registers exactly the known renderer to main dictation channels, each spelled plainly', () => {
    const registrations = [...main.matchAll(IPC_REGISTRATION)];
    const dictation = registrations.filter((match) => /dictation/i.test(match[3]));
    // One spelling, so the scan below cannot miss a channel written in other quotes.
    for (const match of dictation) expect(match[2], match[3]).toBe("'");
    expect(dictation.map((match) => match[3]).sort()).toEqual(
      Object.keys(DICTATION_CHANNELS).sort()
    );
  });

  it('every renderer to main dictation channel first returns early for any other sender', () => {
    const registrations = [...main.matchAll(IPC_REGISTRATION)].filter((match) =>
      match[3].startsWith('dictation:')
    );
    expect(registrations.length).toBe(Object.keys(DICTATION_CHANNELS).length);
    for (const match of registrations) {
      const opening = handlerOpening(main, match);
      expect(opening, match[3]).not.toBeNull();
      // `event` is the callback's own first parameter, so the check reads this call's sender.
      expect(opening!.parameters, match[3]).toMatch(/^, (?:async )?\(event(?:, [^)]*)?\)$/);
      expect(opening!.first, match[3]).toMatch(senderGuard(DICTATION_CHANNELS[match[3]]));
    }
  });

  it('the scan above refuses an argument check before the sender check, or a softer return', () => {
    const opening = (source: string) =>
      handlerOpening(source, [...source.matchAll(IPC_REGISTRATION)][0])?.first ?? '';
    const app = senderGuard(['app']);
    expect(
      opening(
        "ipcMain.handle('dictation:x', (event) => {\n  if (!isDictationSender(event)) return false;"
      )
    ).toMatch(app);
    expect(
      opening(
        "ipcMain.handle('dictation:x', (event) => {\n  if (\n    !isDictationSender(event)\n  )\n    return;"
      )
    ).toMatch(app);
    for (const wrong of [
      // r16's shape: an argument check joined to the sender check.
      "ipcMain.handle('dictation:x', (event, raw) => {\n  if (!isDictationSender(event) || !raw) return false;",
      // An inline window comparison in place of a named check.
      "ipcMain.handle('dictation:x', (event) => {\n  if (!win || event.sender !== win.webContents) return false;",
      // Anything before the check.
      "ipcMain.handle('dictation:x', (event) => {\n  const owner = BrowserWindow.fromWebContents(event.sender);",
      // A return that runs code.
      "ipcMain.handle('dictation:x', (event) => {\n  if (!isDictationSender(event)) return start();",
      // Either sender where both must be refused.
      "ipcMain.handle('dictation:x', (event) => {\n  if (!isDictationSender(event) || !isDictationCaptureSender(event)) return false;",
    ]) {
      expect(opening(wrong), wrong).not.toMatch(app);
    }
    expect(
      opening(
        "ipcMain.handle('dictation:x', (event) => {\n  if (!isDictationSender(event) && !isDictationCaptureSender(event)) return 'unknown';"
      )
    ).toMatch(senderGuard(['app', 'capture']));
  });

  it('the capture and review checks admit only the main frame of the window each names', () => {
    for (const [check, window] of [
      ['isDictationCaptureSender', 'anywhereCaptureWindow'],
      ['isDictationReviewSender', 'anywhereReviewWindow'],
    ] as const) {
      expect(flat(functionText(check)), check).toContain(
        `return Boolean(${window} && !${window}.isDestroyed() && event.sender === ${window}.webContents && event.senderFrame === event.sender.mainFrame);`
      );
    }
    // One check per window, so no second copy can drift from the tested one.
    expect(main).not.toMatch(/function anywhereCaptureSender\(/);
  });

  it('those two windows are only ever ones createAnywhereWindow opened, on this app’s pages', () => {
    const assignments = [
      ...main.matchAll(/\b(anywhere(?:Capture|Review)Window) = (?!=)([^;]+);/g),
    ].map((match) => flat(`${match[1]} = ${match[2]}`));
    expect(new Set(assignments)).toEqual(
      new Set([
        'anywhereCaptureWindow = win',
        'anywhereReviewWindow = win',
        'anywhereCaptureWindow = null',
        'anywhereReviewWindow = null',
        'anywhereCaptureWindow = opened',
        "anywhereReviewWindow = await createAnywhereWindow('dictation-review')",
      ])
    );
    const create = flat(functionText('createAnywhereWindow'));
    expect(create).toContain('const win = new BrowserWindow(');
    expect(create).toContain(
      "if (hash === 'dictation-capture') anywhereCaptureWindow = win; else anywhereReviewWindow = win;"
    );
    // Biorouter's own renderer, and no other page for the window's life.
    expect(create).toContain(
      'const url = rendererEntryUrl(); url.hash = `/${hash}`; await win.loadURL(formatUrl(url));'
    );
    expect(create).toContain("win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));");
    expect(create).toContain(
      "win.webContents.on('will-navigate', (event, target) => { if (!isAppOrigin(target, rendererEntryUrl())) event.preventDefault(); });"
    );
    // `opened` is the capture window that same function opened: the only two writes to
    // `anywhereCaptureOpening` are this call and its own reset.
    const everywhere = flat(main);
    expect(everywhere).toContain('const opened = await anywhereCaptureOpening.catch(() => null);');
    expect(everywhere).toContain(
      "anywhereCaptureOpening ??= createAnywhereWindow('dictation-capture').finally(() => { anywhereCaptureOpening = null; });"
    );
    expect([...main.matchAll(/\banywhereCaptureOpening (?:\?\?)?= (?!=)/g)]).toHaveLength(2);
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
