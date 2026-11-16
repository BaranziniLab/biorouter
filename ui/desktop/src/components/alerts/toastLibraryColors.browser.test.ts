// @vitest-environment node
/**
 * react-toastify paints nothing in its own colours (Red is rare, design.md 9.14; Phase C, T8).
 *
 * An error toast carried the library's red progress bar, `rgb(231, 77, 60)`, under Biorouter's
 * card: hidden (`hideProgressBar` leaves it at opacity 0), but painted, and red. The library's
 * palette lives on `:root` in the vendored stylesheet, imported after `main.css`, so the
 * container re-points every colour inline (`TOAST_LIBRARY_COLORS`). Three claims, each of which
 * fails on its own:
 *   - every colour variable the vendored sheet declares is re-pointed, at a token;
 *   - none of them at one of the reds the app keeps for real danger;
 *   - `App.tsx`'s container applies the map, and in Chromium the container's value is what the
 *     error progress bar paints, not the vendored `:root` one.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { TOAST_LIBRARY_COLORS } from './toastLibraryColors';

const here = dirname(fileURLToPath(import.meta.url));
const VENDORED = readFileSync(
  resolve(here, '../../../node_modules/react-toastify/dist/ReactToastify.css'),
  'utf8'
);
const APP = readFileSync(resolve(here, '../../App.tsx'), 'utf8');
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8').replace(
  /\/\*[\s\S]*?\*\//g,
  ''
);

/** The custom properties the vendored `:root` block declares, with their values. */
function vendoredRoot(): Map<string, string> {
  const start = VENDORED.indexOf(':root');
  const body = VENDORED.slice(VENDORED.indexOf('{', start) + 1, VENDORED.indexOf('}', start));
  return new Map(
    [...body.matchAll(/(--toastify-[a-z0-9-]+)\s*:\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()])
  );
}

/** Whether a vendored declaration is a colour (the progress trail's opacity is a number). */
const isColour = (name: string, value: string) => name.includes('color') && !/^[\d.]+$/.test(value);

const RED_TOKENS = ['--border-danger', '--text-critical', '--background-critical'];

describe('react-toastify paints in tokens', () => {
  it('re-points every colour the vendored sheet declares, at a token and never a red', () => {
    const colours = [...vendoredRoot()].filter(([name, value]) => isColour(name, value));
    // The sheet this test reads is the one the app imports; an empty read would pass vacuously.
    expect(colours.map(([name]) => name)).toContain('--toastify-color-progress-error');
    const ours = TOAST_LIBRARY_COLORS as Record<string, string>;
    for (const [name] of colours) {
      expect(ours[name], `${name} keeps the library's colour`).toMatch(/^var\(--[a-z-]+\)$/);
      for (const red of RED_TOKENS) expect(ours[name]).not.toContain(red);
    }
  });

  it('is applied by the toast container in App.tsx', () => {
    const container = APP.slice(APP.indexOf('<ToastContainer'));
    const style = container.slice(0, container.indexOf('as React.CSSProperties'));
    expect(style).toContain('...TOAST_LIBRARY_COLORS');
  });

  let browser: Browser | null = null;

  beforeAll(async () => {
    for (const options of [{}, { channel: 'chromium' }, { channel: 'chrome' }] as const) {
      try {
        browser = await chromium.launch(options);
        return;
      } catch {
        // try the next build
      }
    }
    console.warn(
      'No Chromium found; skipping the cascade check. Run `npx playwright install chromium`.'
    );
  });

  afterAll(async () => {
    await browser?.close();
  });

  it("the error progress bar paints the container's ink, not the library's red", async (ctx) => {
    if (!browser) return ctx.skip();
    const rootStart = MAIN_CSS.indexOf('\n:root {');
    const muted = MAIN_CSS.slice(rootStart, MAIN_CSS.indexOf('\n}', rootStart)).match(
      /\s--text-muted:\s*([^;]+);/
    )?.[1];
    expect(muted, 'main.css declares --text-muted').toBeTruthy();
    const inline = Object.entries(TOAST_LIBRARY_COLORS)
      .map(([name, value]) => `${name}: ${value}`)
      .join('; ');
    const page = await browser.newPage();
    // Our tokens first, the vendored sheet after them, as the app loads them.
    await page.setContent(`<!doctype html><html><head>
      <style>:root { --text-muted: ${muted}; }</style>
      <style>${VENDORED}</style></head><body>
      <div class="Toastify__toast-container" style="${inline}">
        <div class="Toastify__progress-bar Toastify__progress-bar-theme--light Toastify__progress-bar--error" id="bar"></div>
        <div id="ink" style="background: var(--text-muted)"></div>
      </div></body></html>`);
    const [bar, ink] = await page.evaluate(() => [
      getComputedStyle(document.getElementById('bar')!).backgroundColor,
      getComputedStyle(document.getElementById('ink')!).backgroundColor,
    ]);
    await page.close();
    expect(bar).not.toBe('rgb(231, 77, 60)');
    expect(bar).toBe(ink);
  });
});
