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
 * History bucket, together with the shared `@ws WS-PRIMITIVES` blocks of
 * `main.css` that the row is made of: the `.biorouter-list-row` recipe and the
 * `.br-row-actions` cluster whose reveal this test is about. Nothing here is
 * a stub of either, so a change to the shared reveal reaches this test.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { DUR } from '../../styles/motion';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

/** Every `@ws WS-PRIMITIVES` block of main.css, in file order (the rows, the row actions). */
function primitiveBlocks(): string {
  const out: string[] = [];
  const re =
    /\/\* @ws WS-PRIMITIVES begin: ([^*]+?) \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: \1 \*\//g;
  for (const match of MAIN_CSS.matchAll(re)) out.push(match[2]);
  if (out.length === 0) throw new Error('main.css has no @ws WS-PRIMITIVES block');
  return out.join('\n');
}

/**
 * The app's base focus rule (`:where(a, button, …):focus-visible`, in `@layer base`): the focus
 * ground and an inset 2px edge on the control itself. Pulled from `main.css` so the test sees the
 * rule the row is up against, and it throws rather than going vacuous if the rule moves.
 */
function baseFocusRule(): string {
  const match = MAIN_CSS.match(
    /:where\(\s*:is\(\s*a,\s*button,[\s\S]*?\):focus-visible\s*\{[^}]*\}/
  );
  if (!match) throw new Error('main.css has no base :focus-visible rule to measure against');
  return `@layer base {\n${match[0]}\n}`;
}

const TOKENS = `:root {
  --radius-inner: 4px;
  --radius-md: 8px;
  --radius-element: 8px;
  --row-height: 40px;
  --border-subtle: rgb(200, 200, 200);
  --border-focus: rgb(90, 90, 90);
  --background-canvas: rgb(250, 250, 249);
  --overlay-selected: rgb(220, 220, 220);
  --overlay-hover: rgb(240, 240, 240);
  --background-focus: rgb(235, 235, 235);
  --text-default: rgb(20, 20, 20);
  --dur-fast-min: ${DUR.fastMin}ms;
  --dur-fast: ${DUR.fast}ms;
  --dur-fast-max: ${DUR.fastMax}ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
body { margin: 0; font: 14px/20px sans-serif; }
* { box-sizing: border-box; }
button { font: inherit; border: 0; background: none; padding: 0; }
.text-label { font-size: 14px; line-height: 20px; }
.text-supporting { font-size: 12px; line-height: 16px; }
.truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.min-w-0 { min-width: 0; }
.flex { display: flex; }
.items-center { align-items: center; }
.gap-2 { gap: 8px; }
.sr-only {
  position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
  overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0;
}`;

/**
 * One stat cell in the component's DOM order (`HistoryStat`): the screen reader's label, the
 * figure, then the glyph. `SessionListView.test.tsx` pins that order on the real component, so
 * this mirror and the row it stands for cannot drift apart unnoticed.
 */
const stat = (id: string, figure: string) =>
  `<span class="br-history-stat" id="${id}"><span class="sr-only">Stat: </span><span class="figure">${figure}</span><svg width="14" height="14" style="flex:none"></svg></span>`;

const row = (
  id: string,
  title: string,
  stats: [string, string, string] = ['12', '29,988,671', '5']
) => `
  <div class="biorouter-list-row br-history-row" id="${id}">
    <svg id="${id}-glyph" width="16" height="16" style="flex:none"></svg>
    <button type="button" class="br-history-row-open" id="${id}-open">
      <span class="flex min-w-0 items-center gap-2">
        <span class="text-label truncate" id="${id}-title">${title}</span>
      </span>
      <span class="br-history-row-meta text-supporting"><span>3:18 PM</span><span>· data</span></span>
    </button>
    <div class="br-history-row-stats text-supporting" id="${id}-stats">
      ${stat(`${id}-messages`, stats[0])}
      ${stats[1] ? stat(`${id}-tokens`, stats[1]) : '<span aria-hidden="true"></span>'}
      ${stats[2] ? stat(`${id}-ext`, stats[2]) : '<span aria-hidden="true"></span>'}
    </div>
    <div class="br-row-actions-slot">
      <div class="br-row-actions" data-state="closed" id="${id}-actions">
        <button type="button" style="width:32px;height:32px" id="${id}-rename">R</button>
        <button type="button" style="width:32px;height:32px" id="${id}-more">M</button>
      </div>
    </div>
  </div>`;

const DEFAULT_ROWS = [
  row('a', 'A short chat'),
  row('c', 'A chat with no token figure and no extensions', ['7', '', '']),
  row('d', 'A chat with enormous figures', ['123,456', '1,234,567,890,123', '12']),
  row(
    'b',
    'A chat whose name is long enough that it can never fit beside its stats and actions in a 712 pixel row'
  ),
].join('');

function page(css: string, rows = DEFAULT_ROWS) {
  return `<!doctype html><html><head><style>${TOKENS}\n${baseFocusRule()}\n${primitiveBlocks()}\n${css}</style></head><body>
    <main style="width:760px;padding:0 24px">
      <section>
        <h2 class="br-history-bucket text-supporting" id="bucket">Today</h2>
        <div class="biorouter-list-shell">
          ${rows}
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

  /**
   * Vision V1-R2-6. A modal menu takes the pointer, so `:hover` ends on the row it was opened
   * from. The sidebar's row keeps its ground while its menu is open; History's did not, so
   * the row being acted on was unmarked while its actions showed. The right-click menu marks
   * the row with Radix's `data-state="open"`; the `⋯` menu with the row's own
   * `data-menu-open`.
   */
  it('keeps the hover ground on a row whose menu is open, from either menu', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const ground = (id: string) =>
        tab.evaluate((el) => getComputedStyle(document.getElementById(el)!).backgroundColor, id);
      const settled = (id: string, colour: string) =>
        tab.waitForFunction(
          ([el, want]) => getComputedStyle(document.getElementById(el)!).backgroundColor === want,
          [id, colour] as const
        );
      // Pointer elsewhere, as it is while a menu holds it.
      await tab.mouse.move(990, 590);
      expect(await ground('a')).toBe('rgba(0, 0, 0, 0)');

      await tab.evaluate(() => document.getElementById('a')!.setAttribute('data-state', 'open'));
      await settled('a', 'rgb(240, 240, 240)');
      await tab.evaluate(() => document.getElementById('b')!.setAttribute('data-menu-open', ''));
      await settled('b', 'rgb(240, 240, 240)');
      // Only the row whose menu is open: its neighbours keep the resting ground.
      expect(await ground('c')).toBe('rgba(0, 0, 0, 0)');
      expect(await ground('d')).toBe('rgba(0, 0, 0, 0)');

      // A Radix menu that has closed says `closed`, and the ground goes with it.
      await tab.evaluate(() => document.getElementById('a')!.setAttribute('data-state', 'closed'));
      await settled('a', 'rgba(0, 0, 0, 0)');
      // The same value a hover paints, so the two are one ground and not two near ones.
      await tab.hover('#c-open');
      await settled('c', 'rgb(240, 240, 240)');
    } finally {
      await tab.close();
    }
  });

  /**
   * Vision V1-R3-4. Keyboard focus on the open control drew the base rule's inset ring on the
   * control itself: a 370px text block with no padding, so the ring touched and cut the first
   * letter of the title and of the time and read as a box inside the row's hover fill. The
   * ring is the row's (spec 2.5: `--background-focus` and an inset 2px `--border-focus`), as a
   * sidebar row's is, and the control paints nothing.
   */
  describe('keyboard focus on the open control (V1-R3-4)', () => {
    const RING = 'rgb(90, 90, 90) 0px 0px 0px 2px inset';
    const FOCUS_GROUND = 'rgb(235, 235, 235)';
    const style = (tab: Page, id: string) =>
      tab.evaluate((el) => {
        const cs = getComputedStyle(document.getElementById(el)!);
        return { shadow: cs.boxShadow, ground: cs.backgroundColor };
      }, id);
    const settled = (tab: Page, id: string, ground: string) =>
      tab.waitForFunction(
        ([el, want]) => getComputedStyle(document.getElementById(el)!).backgroundColor === want,
        [id, ground] as const,
        // Short, so a ground that never arrives fails in seconds rather than at the 30s default.
        { timeout: 3000 }
      );
    /** Tab in from the end of the page, the way a keyboard user arrives (so `:focus-visible` holds). */
    const tabTo = async (tab: Page, id: string) => {
      await tab.mouse.move(990, 590);
      await tab.focus('#outside');
      await tab.keyboard.press('Shift+Tab');
      for (let i = 0; i < 12; i++) {
        if ((await tab.evaluate(() => document.activeElement?.id)) === id) return;
        await tab.keyboard.press('Tab');
      }
      throw new Error(`Tab never reached #${id}`);
    };
    /** Where the first glyph of an element's text sits. */
    const textLeft = (tab: Page, selector: string) =>
      tab.evaluate((sel) => {
        const range = document.createRange();
        range.selectNodeContents(document.querySelector(sel)!);
        return range.getBoundingClientRect().left;
      }, selector);

    it('is drawn on the row, clear of the title and the time, and not on the control', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open();
      try {
        // The instrument: the base rule really draws its ring on a bare focused button here, so
        // the control's silence below is the stylesheet's doing and not a missing rule.
        await tabTo(tab, 'outside');
        expect((await style(tab, 'outside')).shadow).toBe(RING);

        await tabTo(tab, 'a-open');
        await settled(tab, 'a', FOCUS_GROUND);
        expect((await style(tab, 'a')).shadow).toBe(RING);
        expect((await style(tab, 'a-open')).shadow).toBe('none');

        // The edge is 2px inside the row; the first letters sit well clear of it (4px is the floor).
        const rowLeft = (await box(tab, 'a')).left;
        expect((await textLeft(tab, '#a-title')) - (rowLeft + 2)).toBeGreaterThanOrEqual(4);
        expect(
          (await textLeft(tab, '#a .br-history-row-meta')) - (rowLeft + 2)
        ).toBeGreaterThanOrEqual(4);

        // The neighbours stay at rest.
        expect((await style(tab, 'b')).shadow).toBe('none');
        expect((await style(tab, 'c')).shadow).toBe('none');
      } finally {
        await tab.close();
      }
    });

    it('still reads as focused when the pointer is over the row', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open();
      try {
        await tabTo(tab, 'a-open');
        await tab.hover('#a-open');
        await settled(tab, 'a', FOCUS_GROUND);
        expect((await style(tab, 'a')).shadow).toBe(RING);
      } finally {
        await tab.close();
      }
    });

    it('leaves nothing behind after a click and keeps the actions their own ring', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open();
      try {
        // A clicked button is not `:focus-visible`: no ring on the row or the control.
        await tab.click('#a-open');
        expect((await style(tab, 'a')).shadow).toBe('none');
        expect((await style(tab, 'a-open')).shadow).toBe('none');

        // Rename keeps the control's own ring; the row draws none and holds only the wash.
        await tabTo(tab, 'a-rename');
        expect((await style(tab, 'a-rename')).shadow).toBe(RING);
        expect((await style(tab, 'a')).shadow).toBe('none');
        await settled(tab, 'a', 'rgb(240, 240, 240)');
      } finally {
        await tab.close();
      }
    });
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

  /** A stat cell's own box, its glyph and the ink of its figure. */
  const statCell = (tab: Page, id: string) =>
    tab.evaluate((el) => {
      const node = document.getElementById(el)!;
      const r = node.getBoundingClientRect();
      const glyph = node.querySelector('svg')!.getBoundingClientRect();
      const figure = node.querySelector('.figure')!.getBoundingClientRect();
      return {
        left: r.left,
        right: r.right,
        width: r.width,
        glyphLeft: glyph.left,
        glyphRight: glyph.right,
        figureLeft: figure.left,
        figureRight: figure.right,
        figureWidth: figure.width,
      };
    }, id);

  it('reads the stats as three fixed columns, whatever a row holds', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // Row `a` holds every figure (a 10-digit token count); row `c` holds only
      // the message count. The message column must not move, its glyph sits on
      // the column's right edge, and its figures end at one x.
      const a = await statCell(tab, 'a-messages');
      const c = await statCell(tab, 'c-messages');
      expect(c.left).toBe(a.left);
      expect(c.width).toBe(a.width);
      expect(c.figureRight).toBe(a.figureRight);
      expect(a.glyphRight).toBeCloseTo(a.right, 0);
      // The token column of the full row sits after it, 12px on, with its glyph
      // on that cell's right edge.
      const tokens = await statCell(tab, 'a-tokens');
      expect(tokens.left - a.right).toBe(12);
      expect(tokens.glyphRight).toBeCloseTo(tokens.right, 0);
    } finally {
      await tab.close();
    }
  });

  /**
   * Vision V1-R6-2. With the glyph before its figure, the glyph moved with the figure's width:
   * the message glyph at x=633, 634 or 639 and the token glyph at 711 or 717, so the glyphs
   * did not form columns. The figures here are the review's own rows (one digit or two, five
   * digits or six, a lower bound), plus a row with no token figure and no extensions.
   */
  it('lines every glyph up in its column and every figure on one edge, whatever its digits', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await browser!.newPage({
      viewport: { width: 1000, height: 600 },
      reducedMotion: 'reduce',
    });
    try {
      const figures: [string, string, string][] = [
        ['14', '168,273', '12'],
        ['8', '84,348', '7'],
        ['24', '292,745', '12'],
        ['4', '≥4,096', '7'],
        ['2', '24,676', '7'],
        ['7', '', ''],
      ];
      const ids = figures.map((_, i) => `r${i}`);
      await tab.setContent(
        page(css, figures.map((stats, i) => row(ids[i], `Chat ${i + 1}`, stats)).join(''))
      );
      for (const stat of ['messages', 'tokens', 'ext']) {
        const cells = [];
        for (const id of ids) {
          if (await tab.$(`#${id}-${stat}`)) cells.push(await statCell(tab, `${id}-${stat}`));
        }
        // The instrument: the figures really differ in width, so equal edges below are the
        // stylesheet's doing and not identical text.
        expect(new Set(cells.map((cell) => cell.figureWidth)).size, stat).toBeGreaterThan(1);
        expect(new Set(cells.map((cell) => cell.glyphLeft)), stat).toEqual(
          new Set([cells[0].glyphLeft])
        );
        expect(new Set(cells.map((cell) => cell.figureRight)), stat).toEqual(
          new Set([cells[0].figureRight])
        );
      }
    } finally {
      await tab.close();
    }
  });

  /**
   * Vision V1-L1: with each glyph at its cell's LEFT edge, the token glyph sat
   * 12px after the message count and 57px before its own value, so it read as
   * part of the message column. Each glyph sits 4px after its own figure, and
   * is further from the next column's figure than from its own.
   */
  it('puts each glyph directly after its own figure', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      for (const row of ['a', 'c', 'd']) {
        for (const stat of ['messages', 'tokens', 'ext']) {
          const id = `${row}-${stat}`;
          if (!(await tab.$(`#${id}`))) continue;
          const { glyphLeft, figureRight } = await statCell(tab, id);
          expect(glyphLeft - figureRight, id).toBeCloseTo(4, 0);
        }
        if (row === 'c') continue;
        const messages = await statCell(tab, `${row}-messages`);
        const tokens = await statCell(tab, `${row}-tokens`);
        const extensions = await statCell(tab, `${row}-ext`);
        expect(tokens.figureLeft - messages.glyphRight, row).toBeGreaterThan(
          messages.glyphLeft - messages.figureRight
        );
        expect(extensions.figureLeft - tokens.glyphRight, row).toBeGreaterThan(
          tokens.glyphLeft - tokens.figureRight
        );
      }
    } finally {
      await tab.close();
    }
  });

  it('grows a track for an outsized figure instead of painting it over its glyph', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      for (const id of ['d-messages', 'd-tokens', 'd-ext']) {
        const { glyphLeft, figureRight } = await statCell(tab, id);
        expect(glyphLeft - figureRight, id).toBeGreaterThanOrEqual(0);
      }
    } finally {
      await tab.close();
    }
  });
});
