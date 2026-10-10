// @vitest-environment node
/**
 * Chat history's rows, measured in a real layout engine (spec 5.7's pattern).
 *
 * jsdom lays nothing out and never loads `history.css`, so the component tests
 * can only see class names. What the stylesheet is FOR is behaviour a person
 * sees: the row's two actions are invisible at rest and appear on hover and on
 * keyboard focus (principle 6, opacity only, so they stay in the tab order),
 * the bucket label sits on the glyph column, and a long title truncates
 * instead of pushing the stats out of their columns.
 *
 * The real `history.css` is loaded into Chromium over markup shaped like a
 * History bucket, with a stub of the shared `.biorouter-list-row` recipe.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { DUR } from '../../styles/motion';

const here = dirname(fileURLToPath(import.meta.url));

const TOKENS = `:root {
  --radius-inner: 4px;
  --radius-element: 8px;
  --row-height: 40px;
  --border-subtle: rgb(200, 200, 200);
  --background-canvas: rgb(250, 250, 249);
  --overlay-selected: rgb(220, 220, 220);
  --text-default: rgb(20, 20, 20);
  --dur-fast-min: ${DUR.fastMin}ms;
  --dur-fast: ${DUR.fast}ms;
  --dur-fast-max: ${DUR.fastMax}ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
body { margin: 0; font: 14px/20px sans-serif; }
* { box-sizing: border-box; }
button { font: inherit; border: 0; background: none; padding: 0; }
/* The shared list-row recipe (WS-PRIMITIVES), reduced to what geometry reads. */
.biorouter-list-row { min-height: var(--row-height); border-bottom: 1px solid var(--border-subtle); }
.text-label { font-size: 14px; line-height: 20px; }
.text-supporting { font-size: 12px; line-height: 16px; }
.truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.min-w-0 { min-width: 0; }
.flex { display: flex; }
.items-center { align-items: center; }
.gap-2 { gap: 8px; }`;

const row = (id: string, title: string) => `
  <div class="biorouter-list-row br-history-row" id="${id}">
    <svg id="${id}-glyph" width="16" height="16" style="flex:none"></svg>
    <button type="button" class="br-history-row-open" id="${id}-open">
      <span class="flex min-w-0 items-center gap-2">
        <span class="text-label truncate" id="${id}-title">${title}</span>
      </span>
      <span class="br-history-row-meta text-supporting"><span>3:18 PM</span><span>· data</span></span>
    </button>
    <div class="br-history-row-stats text-supporting" id="${id}-stats">
      <span>12</span><span>29,988,671</span><span>5</span>
    </div>
    <div class="br-history-row-actions" data-state="closed" id="${id}-actions">
      <button type="button" style="width:32px;height:32px" id="${id}-rename">R</button>
      <button type="button" style="width:32px;height:32px" id="${id}-more">M</button>
    </div>
  </div>`;

function page(css: string) {
  return `<!doctype html><html><head><style>${TOKENS}\n${css}</style></head><body>
    <main style="width:760px;padding:0 24px">
      <section>
        <h2 class="br-history-bucket text-supporting" id="bucket">Today</h2>
        <div class="biorouter-list-shell">
          ${row('a', 'A short chat')}
          ${row('b', 'A chat whose name is long enough that it can never fit beside its stats and actions in a 712 pixel row')}
        </div>
      </section>
    </main>
    <button type="button" id="outside">outside</button>
  </body></html>`;
}

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

describe('Chat history rows in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome) — skipping the History row geometry ' +
          'tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = readFileSync(resolve(here, 'history.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    // Reduced motion: the reveal's transition is off, so a read right after the
    // hover sees the end state rather than a frame of the fade.
    const tab = await browser!.newPage({
      viewport: { width: 1000, height: 600 },
      reducedMotion: 'reduce',
    });
    await tab.setContent(page(css));
    return tab;
  }

  const opacity = (tab: Page, id: string) =>
    tab.evaluate((el) => getComputedStyle(document.getElementById(el)!).opacity, id);
  const box = (tab: Page, id: string) =>
    tab.evaluate((el) => {
      const r = document.getElementById(el)!.getBoundingClientRect();
      return { left: r.left, right: r.right, top: r.top, height: r.height, width: r.width };
    }, id);

  it('hides the actions at rest and shows them on hover, one row at a time', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      expect(await opacity(tab, 'a-actions')).toBe('0');
      expect(await opacity(tab, 'b-actions')).toBe('0');

      await tab.hover('#a-open');
      expect(await opacity(tab, 'a-actions')).toBe('1');
      expect(await opacity(tab, 'b-actions')).toBe('0');
    } finally {
      await tab.close();
    }
  });

  it('shows them to the keyboard too: focus inside the row reveals both', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.mouse.move(990, 590);
      await tab.focus('#b-open');
      expect(await opacity(tab, 'b-actions')).toBe('1');
      // Hidden actions are still in the tab order: opacity only, never display.
      await tab.keyboard.press('Tab');
      expect(await tab.evaluate(() => document.activeElement?.id)).toBe('b-rename');
    } finally {
      await tab.close();
    }
  });

  it('keeps them shown while the row menu is open', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.evaluate(() =>
        document.getElementById('a-actions')!.setAttribute('data-state', 'open')
      );
      await tab.focus('#outside');
      expect(await opacity(tab, 'a-actions')).toBe('1');
    } finally {
      await tab.close();
    }
  });

  it('fades the reveal over --dur-fast-min when motion is allowed', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await browser!.newPage({ reducedMotion: 'no-preference' });
    try {
      await tab.setContent(page(css));
      const transition = await tab.evaluate(
        () => getComputedStyle(document.getElementById('a-actions')!).transition
      );
      expect(transition).toContain('opacity');
      expect(transition).toContain(`${DUR.fastMin / 1000}s`);
    } finally {
      await tab.close();
    }
  });

  it('puts the bucket label on the glyph column and the title 24px after it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const rowBox = await box(tab, 'a');
      const glyph = await box(tab, 'a-glyph');
      const title = await box(tab, 'a-title');
      const bucket = await tab.evaluate(() => {
        const label = document.getElementById('bucket')!;
        const range = document.createRange();
        range.selectNodeContents(label);
        return range.getBoundingClientRect().left;
      });
      expect(glyph.left - rowBox.left).toBe(12);
      expect(bucket).toBe(glyph.left);
      expect(title.left - glyph.left).toBe(24);
    } finally {
      await tab.close();
    }
  });

  it('truncates a long title rather than squeezing the stats or the actions', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const short = await box(tab, 'a-stats');
      const long = await box(tab, 'b-stats');
      expect(long.width).toBe(short.width);
      expect(long.left).toBe(short.left);
      const truncated = await tab.evaluate(() => {
        const title = document.getElementById('b-title')!;
        return title.scrollWidth > title.clientWidth;
      });
      expect(truncated).toBe(true);
      const actions = await box(tab, 'b-actions');
      expect(actions.right).toBeLessThanOrEqual((await box(tab, 'b')).right - 12);
    } finally {
      await tab.close();
    }
  });
});
