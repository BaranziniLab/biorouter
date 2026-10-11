// @vitest-environment node
/**
 * Showing a row that has just moved, measured in a real layout engine (V1-R6-1, ruling 16).
 *
 * jsdom computes no layout, so the scroll arithmetic in `historyReveal.ts` can only be seen here.
 * The real `history.css` is loaded (the sticky bucket label is the reason the arithmetic is not a
 * plain `scrollIntoView`), over a scroller shaped like the History pane: a padded column, buckets
 * with a sticky label, rows with the `data-session-id` the helper finds them by.
 *
 * The helper's own source is injected into the page, so what runs in Chromium is what ships.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { scrollHistoryRowIntoView } from './historyReveal';

const here = dirname(fileURLToPath(import.meta.url));

const ROW = 56;

const rows = (prefix: string, count: number) =>
  Array.from(
    { length: count },
    (_, index) =>
      `<div data-session-id="${prefix}${index + 1}" id="${prefix}${index + 1}" style="height:${ROW}px">${prefix}${index + 1}</div>`
  ).join('');

const markup = (css: string) => `<!doctype html><html><head><style>
  :root { --background-canvas: rgb(250, 250, 249); }
  body { margin: 0; font: 12px/16px sans-serif; }
  * { box-sizing: border-box; }
  #viewport { height: 320px; overflow-y: auto; position: relative; }
  #content { padding: 8px 24px 24px; }
  ${css}
</style></head><body>
  <div id="viewport"><div id="content">
    <section id="s-today"><h2 class="br-history-bucket" id="l-today">Today</h2><div>${rows('t', 3)}</div></section>
    <section id="s-week"><h2 class="br-history-bucket" id="l-week">Previous 7 days</h2><div>${rows('w', 8)}</div></section>
  </div></div>
</body></html>`;

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

describe('scrollHistoryRowIntoView in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome), skipping the History reveal geometry ' +
          'tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = readFileSync(resolve(here, 'history.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 400 } });
    await tab.setContent(markup(css));
    return tab;
  }

  /** Scroll the pane to `top`, run the shipped helper for `id`, and measure the result. */
  async function reveal(tab: Page, top: number, id: string) {
    return tab.evaluate(
      ({ top, id, source }) => {
        const viewport = document.getElementById('viewport')!;
        viewport.scrollTop = top;
        const row = document.getElementById(id)!;
        const scroll = new Function(`return ${source}`)() as (
          v: HTMLElement,
          r: HTMLElement
        ) => void;
        scroll(viewport, row);
        const box = viewport.getBoundingClientRect();
        const at = row.getBoundingClientRect();
        const label = row.closest('section')!.querySelector('h2')!.getBoundingClientRect();
        return {
          scrollTop: viewport.scrollTop,
          rowTop: at.top - box.top,
          rowBottom: at.bottom - box.top,
          labelTop: label.top - box.top,
          labelBottom: label.bottom - box.top,
          height: box.height,
        };
      },
      { top, id, source: scrollHistoryRowIntoView.toString() }
    );
  }

  it('takes a list scrolled far down back to the very top for the first row', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const result = await reveal(tab, 300, 't1');
      expect(result.scrollTop).toBe(0);
      expect(result.rowTop).toBeGreaterThanOrEqual(0);
    } finally {
      await tab.close();
    }
  });

  it('brings the heading with a row that opens its bucket', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // Scrolled past the Today bucket: the first row of "Previous 7 days" is above the pane.
      const result = await reveal(tab, 400, 'w1');
      expect(result.labelTop).toBeCloseTo(0, 0);
      expect(result.rowTop).toBeGreaterThanOrEqual(result.labelBottom - 0.5);
    } finally {
      await tab.close();
    }
  });

  it('keeps a row below the first clear of the sticky heading', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const result = await reveal(tab, 400, 'w3');
      // The heading pins to the top edge; the row starts at or below its bottom.
      expect(result.rowTop).toBeGreaterThanOrEqual(result.labelBottom - 0.5);
      expect(result.rowTop).toBeLessThan(result.labelBottom + 1);
    } finally {
      await tab.close();
    }
  });

  it('brings in a row that is below the pane by the least it takes', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const result = await reveal(tab, 0, 'w7');
      expect(result.rowBottom).toBeCloseTo(result.height, 0);
    } finally {
      await tab.close();
    }
  });

  it('does not scroll for a row that is already in view', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const result = await reveal(tab, 0, 't2');
      expect(result.scrollTop).toBe(0);
    } finally {
      await tab.close();
    }
  });
});
