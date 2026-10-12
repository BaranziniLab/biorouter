// @vitest-environment node
/**
 * Fills before lines keep their shape in forced colours (owner message 13, R-27; design.md 9.5).
 *
 * A filled card, button, chip or field has no resting outline: the fill alone says "there is a
 * box here". Forced colours (Windows High Contrast) repaint every background as Canvas, so the
 * fill vanishes there. The rule in `main.css` and design.md is that a filled control KEEPS its
 * 1px border and makes it transparent (`border border-transparent`), and forced colours draw that
 * border in the system ink. That is a claim about the engine, so it is measured in Chromium with
 * forced colours emulated, against the real token values read from `main.css`:
 *
 *   - at rest the transparent border paints nothing and the box is its fill, a visible step off
 *     the page in both modes;
 *   - in forced colours the fill becomes Canvas and the transparent border is drawn in a visible
 *     system colour, while a box that dropped its border (`border-0`) is left with no edge at all;
 *   - the box is the same size in both, so switching the look costs no geometry.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, 'main.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');

/** The value of `--name` inside the first block that `opener` starts (`:root {` or `.dark {`). */
function baseToken(opener: string, name: string): string {
  const start = MAIN_CSS.indexOf(`\n${opener}`);
  if (start < 0) throw new Error(`main.css has no ${opener} block`);
  const body = MAIN_CSS.slice(start, MAIN_CSS.indexOf('\n}', start));
  const match = body.match(new RegExp(`\\s${name}:\\s*([^;]+);`));
  if (!match) throw new Error(`${opener} does not declare ${name}`);
  return match[1].trim();
}

const MODES = {
  light: {
    page: baseToken(':root {', '--background-default'),
    fill: baseToken(':root {', '--background-fill'),
  },
  dark: {
    page: baseToken('.dark {', '--background-default'),
    fill: baseToken('.dark {', '--background-fill'),
  },
};

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

const html = (mode: keyof typeof MODES) => `<!doctype html><html><head><style>
  body { margin: 0; padding: 24px; background: ${MODES[mode].page}; }
  .box { width: 120px; height: 32px; margin-bottom: 8px; background-color: ${MODES[mode].fill}; }
  .kept { border: 1px solid transparent; }
  .dropped { border: 0; }
</style></head><body><div class="box kept" id="kept"></div><div class="box dropped" id="dropped"></div></body></html>`;

type Measure = {
  canvas: string;
  page: string;
  keptBackground: string;
  keptEdge: string;
  keptEdgeWidth: string;
  keptSize: [number, number];
  droppedEdgeWidth: string;
};

const measure = (page: Page): Promise<Measure> =>
  page.evaluate(() => {
    const cs = (el: Element) => getComputedStyle(el);
    const probe = document.createElement('div');
    probe.style.backgroundColor = 'Canvas';
    document.body.append(probe);
    const canvas = cs(probe).backgroundColor;
    probe.remove();
    const kept = document.getElementById('kept')!;
    const dropped = document.getElementById('dropped')!;
    const box = kept.getBoundingClientRect();
    return {
      canvas,
      page: cs(document.body).backgroundColor,
      keptBackground: cs(kept).backgroundColor,
      keptEdge: cs(kept).borderTopColor,
      keptEdgeWidth: cs(kept).borderTopWidth,
      keptSize: [box.width, box.height] as [number, number],
      droppedEdgeWidth: cs(dropped).borderTopWidth,
    };
  });

/** True when a computed colour paints nothing (`transparent` or any zero alpha). */
const isClear = (colour: string) =>
  colour === 'transparent' || /^rgba\(.*,\s*0\)$/.test(colour.replace(/\s+/g, ' '));

describe('a filled box keeps its shape in forced colours', () => {
  let browser: Browser | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found — skipping the fill token tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
  });

  afterAll(async () => {
    await browser?.close();
  });

  it.for(Object.keys(MODES) as (keyof typeof MODES)[])('%s', async (mode, ctx) => {
    if (!browser) return ctx.skip();
    const page = await browser.newPage();
    try {
      await page.setContent(html(mode));
      const rest = await measure(page);
      expect(isClear(rest.keptEdge), `${mode}: the resting border paints nothing`).toBe(true);
      expect(rest.keptBackground, `${mode}: the box is its fill, not the page`).not.toBe(rest.page);
      expect(rest.keptEdgeWidth).toBe('1px');

      await page.emulateMedia({ forcedColors: 'active' });
      const forced = await measure(page);
      expect(forced.keptBackground, `${mode}: forced colours repaint the fill`).toBe(forced.canvas);
      expect(isClear(forced.keptEdge), `${mode}: the kept border is drawn`).toBe(false);
      expect(forced.keptEdge, `${mode}: the kept border is not Canvas`).not.toBe(forced.canvas);
      expect(forced.keptEdgeWidth).toBe('1px');
      expect(forced.droppedEdgeWidth, `${mode}: a dropped border leaves no edge`).toBe('0px');
      expect(forced.keptSize, `${mode}: no geometry moves`).toEqual(rest.keptSize);
    } finally {
      await page.close();
    }
  });
});
