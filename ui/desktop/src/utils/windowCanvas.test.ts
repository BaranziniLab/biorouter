import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { THEME_FAMILY_IDS } from '../styles/themes.generated';
import {
  WINDOW_CANVAS,
  initialWindowCanvas,
  isWindowCanvasMode,
  type WindowCanvasMode,
} from './windowCanvas';

/**
 * A chat window must never show a colour the app does not paint.
 *
 * The native window background is what the screen shows wherever the renderer's
 * last frame does not reach — for a few frames of every live resize, and for as
 * long as a loaded GPU process is late. It was the `vibrancy: 'window'` material
 * over Electron's default `#FFF`, so a dark app showed a white band on every
 * resize under load (measured: docs/desktop-ui/window-scaling-regressions.md,
 * "Unpainted window area").
 *
 * ⚠ **None of this is visible to jsdom**, and no component test can see it: there
 * is no window, no compositor and no late frame here. So the facts are read from
 * the shipped source — `main.ts` cannot be imported at all (it imports Electron
 * at the top), and the canvas colour lives in `main.css`. The renderer half (the
 * report on mount, on click, on an OS flip) is behavioural, in
 * contexts/ThemeContext.windowCanvas.test.tsx.
 */

describe('initialWindowCanvas', () => {
  it('uses the theme the app last showed, whatever the OS says', () => {
    expect(initialWindowCanvas('dark', false)).toBe(WINDOW_CANVAS.dark);
    expect(initialWindowCanvas('light', true)).toBe(WINDOW_CANVAS.light);
  });

  it('follows the OS on a first launch, like the page does', () => {
    expect(initialWindowCanvas(undefined, true)).toBe(WINDOW_CANVAS.dark);
    expect(initialWindowCanvas(undefined, false)).toBe(WINDOW_CANVAS.light);
  });

  // settings.json is a file on disk: anything can be in it.
  it.each([null, '', 'system', 'DARK', '#131312', 1, {}])(
    'treats a remembered %j as nothing remembered',
    (junk) => {
      expect(initialWindowCanvas(junk, true)).toBe(WINDOW_CANVAS.dark);
      expect(initialWindowCanvas(junk, false)).toBe(WINDOW_CANVAS.light);
    }
  );
});

describe('isWindowCanvasMode', () => {
  it('accepts exactly the two modes — the IPC payload is never a colour', () => {
    expect(isWindowCanvasMode('light')).toBe(true);
    expect(isWindowCanvasMode('dark')).toBe(true);
    for (const v of ['system', '#000000', 'rgba(0,0,0,0)', '', null, undefined, 0, ['dark']]) {
      expect(isWindowCanvasMode(v), JSON.stringify(v)).toBe(false);
    }
  });
});

/**
 * ⚠ The whole fix is "the window's fallback IS the page's canvas". The page's
 * canvas is `--background-app` (the body's background, painted edge to edge),
 * declared once per theme family and mode. A value moved there and not in
 * windowCanvas.ts brings the band back in exactly that theme, with every other
 * test green — so every declaration is resolved and compared, and every family
 * the app ships must be among them.
 */
describe('WINDOW_CANVAS is the canvas main.css paints', () => {
  const css = readFileSync(join(__dirname, '../styles/main.css'), 'utf8').replace(
    /\/\*[\s\S]*?\*\//g,
    ''
  );

  /** Every top-level `selector { … }` block with its custom-property declarations. */
  const blocks = [...css.matchAll(/(^|\n)([^{}\n][^{}]*?)\{([^{}]*)\}/g)].map((m) => ({
    // The selector is the last line before the brace; lines above it are the
    // at-rule statements (`@import`, `@custom-variant`) the pattern swept up.
    selector: m[2].trim().split('\n').pop()!.trim(),
    decls: Object.fromEntries(
      [...m[3].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].map((d) => [d[1], d[2].trim()])
    ),
  }));
  // What an unqualified `var()` falls back to: Tailwind's `@theme` scale (emitted
  // on :root) and the hand-authored bare `:root` block.
  const globals: Record<string, string> = Object.assign(
    {},
    ...blocks.filter((b) => b.selector === '@theme' || b.selector === ':root').map((b) => b.decls)
  );

  const resolve = (value: string, own: Record<string, string>, depth = 0): string => {
    const ref = /^var\((--[\w-]+)\)$/.exec(value);
    if (!ref) return value.toLowerCase();
    expect(depth, `var() chain too deep resolving ${value}`).toBeLessThan(8);
    const next = own[ref[1]] ?? globals[ref[1]];
    expect(next, `${ref[1]} is not declared`).toBeDefined();
    return resolve(next!, own, depth + 1);
  };

  const declarations = blocks
    .filter((b) => b.decls['--background-app'])
    .map((b) => {
      const mode: WindowCanvasMode = /^\.dark\b/.test(b.selector) ? 'dark' : 'light';
      const family = /data-theme='([\w-]+)'/.exec(b.selector)?.[1] ?? 'parchment';
      return { ...b, mode, family, canvas: resolve(b.decls['--background-app'], b.decls) };
    });

  it('finds the base blocks the page is painted from', () => {
    expect(globals['--color-white'], 'the colour scale is missing from main.css').toBeDefined();
    // The body is what paints the canvas edge to edge; without it there is no
    // "page canvas" for the window to match.
    expect(css).toMatch(/\nbody\s*\{[^}]*background-color:\s*var\(--background-app\)/);
  });

  it.each(['light', 'dark'] as const)(
    'declares a %s canvas for every theme family the app ships',
    (mode) => {
      const families = declarations.filter((d) => d.mode === mode).map((d) => d.family);
      for (const family of THEME_FAMILY_IDS) {
        expect(families, `no ${mode} --background-app for ${family}`).toContain(family);
      }
    }
  );

  it('resolves every --background-app to the window canvas of its mode', () => {
    expect(declarations.length).toBeGreaterThanOrEqual(THEME_FAMILY_IDS.length * 2);
    for (const d of declarations) {
      expect(d.canvas, `${d.selector} --background-app`).toBe(WINDOW_CANVAS[d.mode]);
    }
  });
});

/**
 * The main-process half. Each assertion is one option or one guard in `main.ts`
 * whose loss brings the band back or breaks another window, and none of them can
 * fail anywhere but here.
 */
describe('the main-process half, read from main.ts and preload.ts', () => {
  const strip = (text: string) =>
    text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:'"`])\/\/[^\n]*/g, '$1');
  const main = strip(readFileSync(join(__dirname, '../main.ts'), 'utf8'));
  const preload = strip(readFileSync(join(__dirname, '../preload.ts'), 'utf8'));

  const slice = (source: string, marker: string, end: string) => {
    const start = source.indexOf(marker);
    expect(start, `${marker} missing`).toBeGreaterThan(-1);
    const stop = source.indexOf(end, start);
    expect(stop, `${marker} never closed by ${JSON.stringify(end)}`).toBeGreaterThan(start);
    return source.slice(start, stop);
  };

  /** The chat window's options: from its constructor to its webPreferences. */
  const chatWindowOptions = () =>
    slice(main, 'const mainWindow = new BrowserWindow({', 'webPreferences: {');

  it('gives the chat window an opaque background that is the app canvas', () => {
    expect(chatWindowOptions()).toMatch(
      /backgroundColor:\s*initialWindowCanvas\(\s*loadSettings\(\)\.windowCanvasMode,\s*nativeTheme\.shouldUseDarkColors\s*\)/
    );
  });

  // ⚠ Either of these puts a colour the app never paints back behind the page:
  // the material over the background, or no background at all.
  it('gives the chat window no vibrancy and no transparency', () => {
    expect(chatWindowOptions()).not.toMatch(/vibrancy/);
    expect(chatWindowOptions()).not.toMatch(/transparent/);
    // The launcher keeps both on purpose — it is a floating chip — so the
    // assertions above are reading the chat window, not a file that lost them.
    expect(slice(main, 'const launcherWindow = new BrowserWindow({', '});')).toMatch(/vibrancy:/);
  });

  const handler = () => slice(main, "ipcMain.on('set-window-canvas'", '\n  });');

  it('lets a renderer set only one of the two canvases, never a colour', () => {
    const h = handler();
    expect(h).toMatch(/if \(!isWindowCanvasMode\(mode\)\) return;/);
    expect(h).toMatch(/setBackgroundColor\(WINDOW_CANVAS\[mode\]\)/);
    expect(h.indexOf('isWindowCanvasMode(mode)')).toBeLessThan(h.indexOf('setBackgroundColor'));
  });

  // The launcher runs the same renderer (and so the same ThemeProvider) in a
  // transparent window; painting it opaque would turn its chip into a rectangle.
  it('paints chat windows only', () => {
    const h = handler();
    expect(h).toMatch(/!windowMap\.has\(win\.id\)\) return;/);
    expect(h.indexOf('windowMap.has(win.id)')).toBeLessThan(h.indexOf('setBackgroundColor'));
  });

  it('remembers the mode so the next window is created with it', () => {
    expect(handler()).toMatch(/settings\.windowCanvasMode = mode;\s*saveSettings\(settings\);/);
  });

  it('exposes the report on the preload bridge under the channel main listens on', () => {
    expect(preload).toMatch(
      /setWindowCanvas:\s*\(mode: 'light' \| 'dark'\) => \{\s*ipcRenderer\.send\('set-window-canvas', mode\);/
    );
  });
});
