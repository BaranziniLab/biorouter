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

/**
 * History's own list containment (`.session-grid`'s paint containment, which clips anything a row
 * or the runs' block draws outside the shell). Pulled from `main.css`; throws if it moves.
 */
function historyContainment(): string {
  const match = MAIN_CSS.match(
    /\/\* @ws WS-HISTORY begin: history list containment \*\/([\s\S]*?)\/\* @ws WS-HISTORY end: history list containment \*\//
  );
  if (!match) throw new Error('main.css has no WS-HISTORY list containment block');
  return match[1];
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
  --text-muted: rgb(99, 92, 84);
  --background-default: rgb(255, 255, 255);
  --background-fill: rgb(244, 244, 242);
  --background-fill-strong: rgb(234, 234, 230);
  --text-secondary: 13px;
  --text-secondary--line-height: 18px;
  --text-secondary--font-weight: 400;
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

/**
 * A subagent run nested under its parent, in the component's DOM (`HistoryRow` with `nested`):
 * `data-depth="1"`, the 13px title and the muted line on ONE line (the time, then the model
 * chip, a neutral Badge), two stat cells and the run's status in the third track, and 28px
 * actions. `SessionListView.nested.test.tsx` pins that shape on the real component.
 */
const nestedRow = (id: string, title: string, model = 'gpt-5.5') => `
  <div class="biorouter-list-row br-history-row" data-depth="1" id="${id}">
    <svg id="${id}-glyph" width="16" height="16" style="flex:none"></svg>
    <button type="button" class="br-history-row-open" id="${id}-open">
      <span class="br-history-row-title truncate" id="${id}-title">${title}</span>
      <span class="br-history-row-meta text-supporting"><span>3:18 PM</span><span class="br-badge-neutral br-history-run-model" style="display:inline-flex;align-items:center;height:20px;padding:0 6px;font-size:12px;line-height:16px" id="${id}-model"><span class="truncate">${model}</span></span></span>
    </button>
    <div class="br-history-row-stats text-supporting" id="${id}-stats">
      ${stat(`${id}-messages`, '9')}
      ${stat(`${id}-tokens`, '412,330')}
      <span class="br-history-stat br-history-run-status" id="${id}-status"><span class="sr-only">Run status: Completed</span><svg width="14" height="14" style="flex:none"></svg></span>
    </div>
    <div class="br-row-actions-slot">
      <div class="br-row-actions" data-state="closed" id="${id}-actions">
        <button type="button" style="width:28px;height:28px" id="${id}-rename">R</button>
        <button type="button" style="width:28px;height:28px" id="${id}-more">M</button>
      </div>
    </div>
  </div>`;

/** A parent, its runs' block (the newest three, then "Show 2 more runs"), then the next chat. */
const NESTED_ROWS = [
  row('p', 'A chat that delegated'),
  `<div class="br-history-children br-fill-surface" role="group" id="p-runs">
    ${nestedRow('k1', 'Search the cohort for matching variants')}
    ${nestedRow('k2', 'Summarise the three trials', 'claude-sonnet-4-5-20250929-with-a-long-suffix')}
    ${nestedRow('k3', 'A run whose name is long enough that it can never fit beside its model, its stats and its actions')}
    <button type="button" class="br-history-more" id="runs-more"><svg width="16" height="16" style="flex:none"></svg><span class="truncate" id="runs-more-label">Show 2 more runs</span></button>
  </div>`,
  row('q', 'The next chat'),
].join('');

/** The same parent with its runs all shown: the block ends on a run, not on "Show more". */
const NESTED_ROWS_EXPANDED = [
  row('p', 'A chat that delegated'),
  `<div class="br-history-children br-fill-surface" role="group" id="p-runs">
    ${nestedRow('k1', 'Search the cohort')}
    ${nestedRow('k2', 'Summarise the trials')}
  </div>`,
  row('q', 'The next chat'),
].join('');

function page(css: string, rows = DEFAULT_ROWS, shellClass = 'biorouter-list-shell') {
  return `<!doctype html><html><head><style>${TOKENS}\n${baseFocusRule()}\n${primitiveBlocks()}\n${historyContainment()}\n${css}</style></head><body>
    <main style="width:760px;padding:0 24px">
      <section>
        <h2 class="br-history-bucket text-supporting" id="bucket">Today</h2>
        <div class="${shellClass}" id="shell">
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
  /**
   * The person's ask: in History a subagent run looked as wide and as heavy as the chat that
   * started it. A run now sits in an inset block under its parent, narrower on BOTH sides, on
   * one line at the 40px floor, with a 13px title, tied to the parent by the rail and an elbow
   * per run, all of it inside the list shell (whose paint containment would clip anything
   * outside it) and none of it widening the page.
   */
  describe('subagent runs nested under their parent', () => {
    const SHELL = 'session-grid biorouter-list-shell br-rows-flat';
    async function openNested(rows = NESTED_ROWS): Promise<Page> {
      const tab = await browser!.newPage({
        viewport: { width: 1000, height: 700 },
        reducedMotion: 'reduce',
      });
      await tab.setContent(page(css, rows, SHELL));
      return tab;
    }
    /** A pseudo-element's box in page coordinates, from its computed offsets and its host's box. */
    const pseudo = (tab: Page, id: string) =>
      tab.evaluate((el) => {
        const host = document.getElementById(el)!;
        const hostBox = host.getBoundingClientRect();
        const hostStyle = getComputedStyle(host);
        // Absolute offsets are from the host's PADDING box, inside its border.
        const originLeft = hostBox.left + parseFloat(hostStyle.borderLeftWidth);
        const originTop = hostBox.top + parseFloat(hostStyle.borderTopWidth);
        const cs = getComputedStyle(host, '::before');
        const left = originLeft + parseFloat(cs.left);
        const top = originTop + parseFloat(cs.top);
        return {
          content: cs.content,
          left,
          right: left + parseFloat(cs.width),
          top,
          bottom: top + parseFloat(cs.height),
        };
      }, id);

    it('draws the runs as a block narrower than the parent on both sides', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openNested();
      try {
        const parent = await box(tab, 'p');
        const block = await box(tab, 'p-runs');
        const shell = await box(tab, 'shell');
        expect(parent.left).toBe(shell.left);
        expect(parent.right).toBe(shell.right);
        // Inset on the left to the parent's title column, and about 16px on the right.
        expect(block.left - parent.left).toBe(27);
        expect(parent.right - block.right).toBe(16);
        // Each run is the block's width, so every run is narrower than its parent too.
        for (const id of ['k1', 'k2', 'k3']) {
          const run = await box(tab, id);
          expect(run.left, id).toBeGreaterThan(parent.left);
          expect(run.right, id).toBeLessThan(parent.right);
        }
        // The block is a fill on the container radius, and its controls step back a ground.
        const ground = await tab.evaluate(() => {
          const cs = getComputedStyle(document.getElementById('p-runs')!);
          const run = getComputedStyle(document.getElementById('k1')!);
          return {
            fill: cs.backgroundColor,
            radius: cs.borderTopLeftRadius,
            control: run.getPropertyValue('--br-control-ground').trim(),
          };
        });
        expect(ground.fill).toBe('rgb(244, 244, 242)');
        expect(ground.radius).toBe('8px');
        expect(ground.control).toBe('rgb(255, 255, 255)');
      } finally {
        await tab.close();
      }
    });

    it("puts each run's glyph under the parent's title, and its stats in their own columns", async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openNested();
      try {
        const parentTitle = await box(tab, 'p-title');
        for (const id of ['k1', 'k2', 'k3']) {
          expect((await box(tab, `${id}-glyph`)).left, id).toBe(parentTitle.left);
        }
        // The runs' three tracks line up with each other, whatever their titles hold, and end
        // where the parent's do: the figures read straight down from a chat into its runs.
        for (const [stat, parentStat] of [
          ['messages', 'messages'],
          ['tokens', 'tokens'],
          ['status', 'ext'],
        ] as const) {
          const rights = new Set<number>();
          for (const id of ['k1', 'k2', 'k3']) rights.add((await box(tab, `${id}-${stat}`)).right);
          expect(rights, stat).toEqual(new Set([(await box(tab, `p-${parentStat}`)).right]));
        }
      } finally {
        await tab.close();
      }
    });

    it('draws a run as one 13px line at the 40px row floor, shorter than its parent', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openNested();
      try {
        const parent = await box(tab, 'p');
        const font = (id: string) =>
          tab.evaluate((el) => {
            const cs = getComputedStyle(document.getElementById(el)!);
            return { size: cs.fontSize, weight: cs.fontWeight };
          }, id);
        for (const id of ['k1', 'k2', 'k3']) {
          const run = await box(tab, id);
          expect(run.height, id).toBe(40);
          expect(run.height, id).toBeLessThan(parent.height);
          expect(await font(`${id}-title`), id).toEqual({ size: '13px', weight: '400' });
          // One line: the title and the model chip share a row.
          const title = await box(tab, `${id}-title`);
          const model = await box(tab, `${id}-model`);
          expect(
            Math.abs(title.top + title.height / 2 - (model.top + model.height / 2))
          ).toBeLessThan(2);
          expect(model.left).toBeGreaterThan(title.left);
        }
        // The parent's title keeps the label size (its weight is a utility this page does not load).
        expect((await font('p-title')).size).toBe('14px');
        // A long title truncates; the chip keeps its cap and the stats keep their place.
        const truncated = await tab.evaluate(() => {
          const title = document.getElementById('k3-title')!;
          return title.scrollWidth > title.clientWidth;
        });
        expect(truncated).toBe(true);
        expect((await box(tab, 'k2-model')).width).toBeLessThanOrEqual(160);
        expect((await box(tab, 'k3-stats')).left).toBe((await box(tab, 'k1-stats')).left);
      } finally {
        await tab.close();
      }
    });

    it('ties the runs to the parent with a rail and an elbow per run, inside the shell', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openNested();
      try {
        const shell = await box(tab, 'shell');
        const block = await box(tab, 'p-runs');
        const parentGlyph = await box(tab, 'p-glyph');
        const rail = await pseudo(tab, 'p-runs');
        expect(rail.content).not.toBe('none');
        // Centred under the parent's glyph, and inside the shell (which clips its paint).
        expect((rail.left + rail.right) / 2).toBe((parentGlyph.left + parentGlyph.right) / 2);
        expect(rail.left).toBeGreaterThanOrEqual(shell.left);
        expect(rail.top).toBeGreaterThanOrEqual(block.top);
        for (const id of ['k1', 'k2', 'k3', 'runs-more']) {
          const host = await box(tab, id);
          const elbow = await pseudo(tab, id);
          expect(elbow.content, id).not.toBe('none');
          // From the rail to the block's edge, at the run's middle.
          expect(elbow.left, id).toBe(rail.left);
          expect(elbow.right, id).toBe(block.left);
          expect(elbow.left, id).toBeGreaterThanOrEqual(shell.left);
          expect((elbow.top + elbow.bottom) / 2, id).toBe(host.top + host.height / 2);
        }
        // The rail stops at the last elbow ("Show 2 more runs" here), so the two read as one
        // bracket and the rail never runs on past what it ties.
        const lastElbow = await pseudo(tab, 'runs-more');
        expect(rail.bottom).toBe(lastElbow.bottom);
      } finally {
        await tab.close();
      }
    });

    it('ends the rail at the last run when every run is shown', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openNested(NESTED_ROWS_EXPANDED);
      try {
        const rail = await pseudo(tab, 'p-runs');
        const lastElbow = await pseudo(tab, 'k2');
        expect(rail.bottom).toBe(lastElbow.bottom);
      } finally {
        await tab.close();
      }
    });

    it('scrolls nothing sideways: the shell and the page hold every run', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openNested();
      try {
        const overflow = await tab.evaluate(() => {
          const shell = document.getElementById('shell')!;
          const block = document.getElementById('p-runs')!;
          return {
            shell: shell.scrollWidth - shell.clientWidth,
            block: block.scrollWidth - block.clientWidth,
            page: document.documentElement.scrollWidth - document.documentElement.clientWidth,
          };
        });
        expect(overflow).toEqual({ shell: 0, block: 0, page: 0 });
        const block = await box(tab, 'p-runs');
        for (const id of ['k1', 'k2', 'k3']) {
          expect((await box(tab, `${id}-actions`)).right, id).toBeLessThanOrEqual(block.right);
        }
      } finally {
        await tab.close();
      }
    });

    it('puts "Show 2 more runs" on the runs\' title column, in the tab order', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openNested();
      try {
        const more = await box(tab, 'runs-more');
        const label = await tab.evaluate(() => {
          const range = document.createRange();
          range.selectNodeContents(document.getElementById('runs-more-label')!);
          return range.getBoundingClientRect().left;
        });
        expect(label).toBe((await box(tab, 'k1-title')).left);
        expect(more.height).toBe(32);
        await tab.focus('#k3-more');
        await tab.keyboard.press('Tab');
        expect(await tab.evaluate(() => document.activeElement?.id)).toBe('runs-more');
        // The app's focus treatment, on the row and not as an outline around it.
        const focus = await tab.evaluate(() => {
          const cs = getComputedStyle(document.getElementById('runs-more')!);
          return { ground: cs.backgroundColor, shadow: cs.boxShadow };
        });
        expect(focus).toEqual({
          ground: 'rgb(235, 235, 235)',
          shadow: 'rgb(90, 90, 90) 0px 0px 0px 2px inset',
        });
      } finally {
        await tab.close();
      }
    });
  });
});
