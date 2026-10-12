// @vitest-environment node
/**
 * The wordmark in forced colours, measured in a real engine (vision round 2, V4-R2-7).
 *
 * jsdom computes no colour and never evaluates `@media (forced-colors: active)`, so
 * `BrandMarks.test.tsx` can only pin the inks as source text. What this checks is what the
 * person sees: the real `BioRouterWordmark` (bundled with esbuild, measured by the browser the way
 * it is in the app, so the bar's two halves exist) is drawn with `styles/main.css`'s
 * `wordmark forced colours` block.
 *
 *   - Outside forced colours the inks are the brand's own, untouched: navy "Bio" and bar half on a
 *     light ground, teal on a dark one, coral "Router" and bar half on both.
 *   - Under `forced-colors: active` Chromium keeps an SVG `fill` it was given as a colour (measured
 *     before the fix: navy at about 1.3:1 on a black canvas), so every drawn part takes
 *     `CanvasText`, the colour the person's palette guarantees against their canvas, and none of
 *     them equals the canvas.
 *   - Painting both halves of the bar one colour must not show the join: the pixels along the bar
 *     are one value end to end (the navy half runs a hair under the coral one).
 *
 * Set `WS_ICONS_SHOT_DIR` to also write a screenshot of each state.
 */
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type BrowserContext, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

/** The `@ws WS-ICONS` block named `name`, so only this component's rule is painted. */
function wsBlock(name: string): string {
  const re = new RegExp(
    `/\\* @ws WS-ICONS begin: ${name} \\*/([\\s\\S]*?)/\\* @ws WS-ICONS end: ${name} \\*/`
  );
  const match = MAIN_CSS.match(re);
  if (!match) throw new Error(`main.css has no "@ws WS-ICONS begin: ${name}" block`);
  return match[1];
}

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { BioRouterWordmark } from './BioRouterWordmark';

createRoot(document.getElementById('root')!).render(
  <div>
    <BioRouterWordmark data-testid="light" dark={false} className="wm" />
    <BioRouterWordmark data-testid="dark" dark className="wm" />
  </div>
);
`;

const launchChromium = async (): Promise<Browser | null> => {
  for (const options of [{}, { channel: 'chromium' }, { channel: 'chrome' }] as const) {
    try {
      return await chromium.launch(options);
    } catch {
      // try the next build
    }
  }
  return null;
};

interface Ink {
  /** Computed fill of each drawn part: the two words, then the two halves of the bar. */
  parts: string[];
  /** Number of `.br-wordmark-ink` nodes inside the svg, so a part losing the class is caught. */
  inked: number;
  canvas: string;
  canvasText: string;
  opacity: string;
}

describe('the wordmark in forced colours (V4-R2-7)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the wordmark forced colours test.'
      );
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css = wsBlock('wordmark forced colours');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Both wordmarks on a page, measured and ready to read. */
  async function open(
    forcedColors: 'none' | 'active',
    colorScheme: 'light' | 'dark',
    deviceScaleFactor = 1
  ): Promise<{ ctx: BrowserContext; page: Page }> {
    const ctx = await browser!.newContext({
      forcedColors,
      colorScheme,
      deviceScaleFactor,
      viewport: { width: 520, height: 260 },
    });
    const page = await ctx.newPage();
    await page.setContent(
      `<!doctype html><html><head><style>
        body { margin: 0; padding: 16px; font: 14px Arial, sans-serif; }
        .wm { display: block; height: 72px; width: auto; margin-bottom: 16px; }
        ${css}
      </style></head><body>
        <div id="root"></div>
        <span id="probe-canvas" style="background-color: Canvas">c</span>
        <span id="probe-text" style="color: CanvasText">t</span>
      </body></html>`
    );
    await page.addScriptTag({ content: script });
    // The wordmark stays at opacity 0 until the browser has measured the text.
    await page.waitForFunction(() =>
      [...document.querySelectorAll('svg.wm')].every((s) => s.getAttribute('opacity') === '1')
    );
    return { ctx, page };
  }

  async function ink(
    forcedColors: 'none' | 'active',
    colorScheme: 'light' | 'dark',
    shot?: string
  ): Promise<Record<'light' | 'dark', Ink>> {
    const { ctx, page } = await open(forcedColors, colorScheme);
    try {
      if (shot && process.env.WS_ICONS_SHOT_DIR) {
        await page.screenshot({ path: join(process.env.WS_ICONS_SHOT_DIR, shot) });
      }
      return await page.evaluate(() => {
        const canvas = getComputedStyle(document.getElementById('probe-canvas')!).backgroundColor;
        const canvasText = getComputedStyle(document.getElementById('probe-text')!).color;
        const read = (testid: string): Ink => {
          const svg = document.querySelector(`svg[data-testid="${testid}"]`)!;
          const parts = [
            ...svg.querySelectorAll('text > tspan'),
            ...svg.querySelectorAll(':scope > rect'),
          ];
          return {
            parts: parts.map((p) => getComputedStyle(p).fill),
            inked: svg.querySelectorAll('.br-wordmark-ink').length,
            canvas,
            canvasText,
            opacity: svg.getAttribute('opacity') ?? '',
          };
        };
        return { light: read('light'), dark: read('dark') };
      });
    } finally {
      await ctx.close();
    }
  }

  /**
   * The grey value of every device pixel along the middle of the first wordmark's bar, read back
   * from a real screenshot (decoded in the page, because the test has no PNG decoder).
   */
  async function barRow(
    forcedColors: 'none' | 'active',
    colorScheme: 'light' | 'dark',
    deviceScaleFactor: number
  ): Promise<number[]> {
    const { ctx, page } = await open(forcedColors, colorScheme, deviceScaleFactor);
    try {
      const bar = await page.evaluate(() => {
        const rects = [...document.querySelectorAll('svg[data-testid="light"] > rect')].map((r) =>
          r.getBoundingClientRect()
        );
        const left = Math.min(...rects.map((r) => r.left));
        const right = Math.max(...rects.map((r) => r.right));
        const top = Math.min(...rects.map((r) => r.top));
        const bottom = Math.max(...rects.map((r) => r.bottom));
        return { left, right, y: (top + bottom) / 2 };
      });
      const png = (await page.screenshot()).toString('base64');
      return await page.evaluate(
        async ({ png, bar, scale }) => {
          const blob = await (await fetch(`data:image/png;base64,${png}`)).blob();
          const bitmap = await createImageBitmap(blob);
          const canvas = document.createElement('canvas');
          canvas.width = bitmap.width;
          canvas.height = bitmap.height;
          const g = canvas.getContext('2d')!;
          g.drawImage(bitmap, 0, 0);
          const y = Math.round(bar.y * scale);
          // One device pixel in from each end: the bar's own anti-aliased edge is not the join.
          const from = Math.ceil(bar.left * scale) + 1;
          const to = Math.floor(bar.right * scale) - 1;
          const row = g.getImageData(from, y, to - from, 1).data;
          const out: number[] = [];
          for (let i = 0; i < row.length; i += 4) out.push(row[i]);
          return out;
        },
        { png, bar, scale: deviceScaleFactor }
      );
    } finally {
      await ctx.close();
    }
  }

  it('draws four inked parts: two words and both halves of the bar', async (ctx) => {
    if (!browser) return ctx.skip();
    const { light, dark } = await ink('none', 'light');
    expect(light.parts).toHaveLength(4);
    expect(light.inked).toBe(4);
    expect(dark.inked).toBe(4);
    expect(light.opacity).toBe('1');
  });

  it('keeps the brand inks outside forced colours: navy on light, teal on dark, coral on both', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const scheme of ['light', 'dark'] as const) {
      const { light, dark } = await ink('none', scheme, `wordmark-normal-${scheme}.png`);
      // [Bio, Router, bar navy half, bar coral half]
      expect(light.parts, `${scheme}: light treatment`).toEqual([
        'rgb(5, 32, 73)',
        'rgb(184, 90, 50)',
        'rgb(5, 32, 73)',
        'rgb(184, 90, 50)',
      ]);
      expect(dark.parts, `${scheme}: dark treatment`).toEqual([
        'rgb(24, 163, 172)',
        'rgb(184, 90, 50)',
        'rgb(24, 163, 172)',
        'rgb(184, 90, 50)',
      ]);
    }
  });

  it('takes every part to CanvasText under forced colours, never the canvas itself', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const scheme of ['light', 'dark'] as const) {
      const result = await ink('active', scheme, `wordmark-forced-${scheme}.png`);
      for (const treatment of ['light', 'dark'] as const) {
        const { parts, canvas, canvasText } = result[treatment];
        expect(canvasText, `${scheme}: the probe resolves CanvasText`).not.toBe(canvas);
        // The navy "Bio" and its bar half are the parts that vanished; the coral ones go with
        // them so the lockup is one ink the person's own palette guarantees.
        expect(parts, `${scheme} ${treatment}: every part is CanvasText`).toEqual([
          canvasText,
          canvasText,
          canvasText,
          canvasText,
        ]);
        for (const fill of parts) expect(fill).not.toBe(canvas);
      }
    }
  });

  it('shows no join in the bar when both halves are one colour', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const scale of [1, 2]) {
      const row = await barRow('active', 'light', scale);
      expect(row.length, `${scale}x: the bar is wider than a few pixels`).toBeGreaterThan(20);
      // Black ink on a white canvas: every pixel along the middle of the bar is solid.
      expect(Math.max(...row), `${scale}x: brightest pixel in the bar ${JSON.stringify(row)}`).toBe(
        Math.min(...row)
      );
    }
  });
});
