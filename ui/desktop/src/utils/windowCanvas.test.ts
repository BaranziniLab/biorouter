import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import ts from 'typescript';
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
  const mainText = readFileSync(join(__dirname, '../main.ts'), 'utf8');
  const main = strip(mainText);
  const preload = strip(readFileSync(join(__dirname, '../preload.ts'), 'utf8'));

  /**
   * ⚠ Parsed, not sliced. This used to cut main.ts from the constructor to the
   * first `webPreferences: {`, so an option written AFTER that block — which is
   * exactly where the launcher's own `vibrancy` sits — was never read, and a
   * `vibrancy` or `transparent` put back there passed every assertion. The
   * TypeScript parser ends the object literal at its real closing brace, whatever
   * is nested in it, and hands back its properties rather than a string to grep.
   */
  const mainAst = ts.createSourceFile('main.ts', mainText, ts.ScriptTarget.Latest, true);
  const collect = <T extends ts.Node>(test: (node: ts.Node) => node is T): T[] => {
    const found: T[] = [];
    const visit = (node: ts.Node) => {
      if (test(node)) found.push(node);
      ts.forEachChild(node, visit);
    };
    visit(mainAst);
    return found;
  };
  const oneline = (node: ts.Node) => node.getText(mainAst).replace(/\s+/g, ' ');

  /** The options object of `const <name> = new BrowserWindow({ … })`, whole. */
  const windowOptions = (name: string): ts.ObjectLiteralExpression => {
    const found = collect(
      (node): node is ts.VariableDeclaration =>
        ts.isVariableDeclaration(node) &&
        ts.isIdentifier(node.name) &&
        node.name.text === name &&
        !!node.initializer &&
        ts.isNewExpression(node.initializer) &&
        node.initializer.expression.getText(mainAst) === 'BrowserWindow'
    );
    expect(found, `exactly one \`const ${name} = new BrowserWindow(…)\``).toHaveLength(1);
    const args = (found[0].initializer as ts.NewExpression).arguments ?? [];
    expect(args, `${name}: one options argument`).toHaveLength(1);
    expect(ts.isObjectLiteralExpression(args[0]), `${name}: options are an object literal`).toBe(
      true
    );
    return args[0] as ts.ObjectLiteralExpression;
  };

  /** Its own top-level entries, by name; a spread is `...<expression>`. */
  const topLevel = (options: ts.ObjectLiteralExpression) =>
    options.properties.map((property) =>
      ts.isSpreadAssignment(property)
        ? `...${property.expression.getText(mainAst)}`
        : property.name!.getText(mainAst).replace(/^['"]|['"]$/g, '')
    );

  const chatWindowOptions = () => windowOptions('mainWindow');

  it('gives the chat window an opaque background that is the app canvas', () => {
    const background = chatWindowOptions().properties.filter(
      (p) => p.name?.getText(mainAst) === 'backgroundColor'
    );
    expect(background, 'exactly one top-level backgroundColor').toHaveLength(1);
    expect(oneline(background[0])).toMatch(
      /^backgroundColor: initialWindowCanvas\( ?loadSettings\(\)\.windowCanvasMode, nativeTheme\.shouldUseDarkColors ?\)$/
    );
  });

  // ⚠ Either of these puts a colour the app never paints back behind the page:
  // the material over the background, or no background at all. Read from the
  // WHOLE options object, before and after webPreferences alike.
  it('gives the chat window no vibrancy and no transparency', () => {
    const keys = topLevel(chatWindowOptions());
    // The object really was read to its end: webPreferences is inside it, and
    // so is what follows it.
    expect(keys).toContain('webPreferences');
    expect(keys).not.toContain('vibrancy');
    expect(keys).not.toContain('transparent');
    // A spread could carry either key in without naming it here.
    expect(keys.filter((k) => k.startsWith('...'))).toEqual([]);
    // Nor put back after construction.
    expect(main).not.toMatch(/\.setVibrancy\(/);
    // The launcher keeps both on purpose — it is a floating chip — and writes
    // `vibrancy` after its webPreferences, so this proves the reader sees an
    // option in exactly the position the old slice could not.
    const launcher = topLevel(windowOptions('launcherWindow'));
    expect(launcher).toContain('vibrancy');
    expect(launcher.indexOf('vibrancy')).toBeGreaterThan(launcher.indexOf('webPreferences'));
  });

  /** The `set-window-canvas` listener, to the balanced end of its call. */
  const handler = () => {
    const found = collect(
      (node): node is ts.CallExpression =>
        ts.isCallExpression(node) &&
        node.expression.getText(mainAst) === 'ipcMain.on' &&
        ts.isStringLiteral(node.arguments[0]) &&
        node.arguments[0].text === 'set-window-canvas'
    );
    expect(found, "exactly one ipcMain.on('set-window-canvas', …)").toHaveLength(1);
    return strip(found[0].getText(mainAst));
  };

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
