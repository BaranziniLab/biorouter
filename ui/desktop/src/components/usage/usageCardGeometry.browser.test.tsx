// @vitest-environment node
/**
 * Home's usage card, measured in a real layout engine (spec 3.9, 5.7's pattern).
 *
 * jsdom lays nothing out, so the component tests can say what the card renders
 * but not how big it is. This renders the real `StatGrid` and `UsageHeatmap` to
 * markup, wraps them in the card's own classes, and lays them out in Chromium
 * with the card's stylesheet (`usage.css`) and the heatmap's block from
 * `main.css` (between its `@ws WS-USAGE` markers) at the token values. It pins
 * what the acceptance names as measurements:
 *  - the card is the 760px chat measure with a 36px head over a 1px hairline, and a FILL
 *    with a transparent 1px edge (R-27): no outline, nothing moves;
 *  - the segmented range control inside it rests one ground deeper than the card (a fill
 *    never sits on a fill; the card opts in with `.br-fill-surface` and the primitives block
 *    of `main.css` does the rest), and an empty heatmap day is a step off the fill in light
 *    and dark;
 *  - six tiles in three columns, two under a 480px container, figures at 24px;
 *  - the heatmap is fluid: one equal column per week, square cells, a 3px gap,
 *    filling the card's 726px inner width; month labels are 12px, in one row, and a label in
 *    the last two columns ends flush with the grid's right edge;
 *  - both tabs keep one height (the panels' 112px floor);
 *  - the tab panels share one grid cell, so a crossfade moves nothing, and the
 *    leaving panel fades out on its own animation (Radix keeps it mounted for it).
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { ActivityWindow } from '../../api';
import { UsageHeatmap } from '../sessions/UsageHeatmap';
import { StatGrid, type StatItem } from './StatGrid';

const here = dirname(fileURLToPath(import.meta.url));

/** `main.css`'s values for the tokens the card and the heatmap read. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, sans-serif;
  --text-supporting: 12px;
  --text-supporting--line-height: 16px;
  --radius-inner: 4px;
  --radius-element: 8px;
  --control-sm: 28px;
  --radius-container: 12px;
  --radius-full: 9999px;
  --dock-height: 36px;
  --border-subtle: #e5e3df;
  --border-focus: #6b6963;
  --background-card: #ffffff;
  --background-default: #ffffff;
  --background-fill: #f4f4f2;
  --background-fill-strong: #eaeae6;
  --background-strong: #dcdcd8;
  --text-default: #1f1e1c;
  --text-muted: #6b6862;
  --heat-0: #eeeeea; --heat-1: #e9c9ab; --heat-2: #dda27a; --heat-3: #c6774c; --heat-4: #a04a27;
  --dur-fast-min: 95ms; --dur-fast: 125ms; --dur-fast-max: 175ms; --dur-med: 300ms; --dur-slow: 525ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
html.dark {
  --border-subtle: #302f2c;
  --background-card: #1b1b19;
  --background-default: #1b1b19;
  --background-fill: #232320;
  --background-fill-strong: #2c2c29;
  --background-strong: #3a3a36;
  --text-default: #f4f0e6;
  --heat-0: #32312e; --heat-1: #573e2a; --heat-2: #7a4d2e; --heat-3: #b0653a; --heat-4: #e8895f;
}
* { box-sizing: border-box; }
body { margin: 0; font: 14px/20px var(--font-body); }
p { margin: 0; }
.text-title { font-size: 24px; line-height: 32px; font-weight: 400; }
.text-body { font-size: 14px; line-height: 20px; }
.text-supporting { font-size: 12px; line-height: 16px; }
.bg-heat-0 { background: var(--heat-0); } .bg-heat-1 { background: var(--heat-1); }
.bg-heat-2 { background: var(--heat-2); } .bg-heat-3 { background: var(--heat-3); }
.bg-heat-4 { background: var(--heat-4); }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
  overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }`;

/** The heatmap's authored block, cut from `main.css` at its owner's markers. */
function heatmapCss(): string {
  const main = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
  const begin = main.indexOf('/* @ws WS-USAGE begin: heatmap */');
  const end = main.indexOf('/* @ws WS-USAGE end: heatmap */');
  if (begin < 0 || end < begin) throw new Error('heatmap block markers not found in main.css');
  return main.slice(begin, end);
}

/**
 * The segmented control's own track rule, cut from `main.css`. It reads `--br-track-ground`, so
 * the real rule is the thing to lay out against: the card's ground reaches it only through the
 * variable below.
 */
function segmentedTrackCss(): string {
  const main = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
  const match = main.match(/\n\.br-segmented \{[^}]*\}/);
  if (!match) throw new Error('.br-segmented rule not found in main.css');
  return match[0];
}

/**
 * The primitives block's filled-box rule, cut from `main.css`: what `.br-fill-surface` hands the
 * controls inside (`--br-track-ground` among them). The card used to restate this by hand.
 */
function fillSurfaceCss(): string {
  const main = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
  const match = main.match(/\n\.br-fill-surface > \*,[^{]*\{[^}]*\}/);
  if (!match) throw new Error('.br-fill-surface rule not found in main.css');
  return match[0];
}

/** A 371-day window, as the daemon serves it. */
const YEAR: ActivityWindow = {
  start: '2025-10-04',
  end: '2026-10-09',
  maxSessions: 1,
  maxTokens: 1,
  tokensComplete: true,
  currentStreak: 0,
  longestStreak: 0,
  days: [],
};

const TILES: StatItem[] = [
  { key: 'chats', label: 'Chats', value: '352' },
  { key: 'messages', label: 'Messages', value: '230,115' },
  { key: 'tokens', label: 'Tokens', value: '87.9B' },
  { key: 'active', label: 'Active days', value: '60', suffix: '/365' },
  { key: 'streak', label: 'Streak', value: '3', suffix: 'days' },
  { key: 'model', label: 'Favorite model', value: 'GPT-5.6 Sol' },
];

/** The card's markup: its classes, a stand-in head, and the real grid and heatmap. */
function cardHtml(width: number, { surface = true } = {}): string {
  const tiles = renderToStaticMarkup(<StatGrid items={TILES} />);
  const heatmap = renderToStaticMarkup(<UsageHeatmap window={YEAR} />);
  return `
    <section class="br-usage-card${surface ? ' br-fill-surface' : ''}" id="card" style="width:${width}px">
      <div class="br-usage-card-head" id="head">
        <div class="br-usage-card-tabs">
          <div role="tablist" id="tablist" style="display:flex;gap:20px">
            <button style="height:36px;font:inherit;border:0;background:none;padding:0">Overview</button>
          </div>
        </div>
        <div class="br-usage-card-range br-segmented" id="range" style="width:96px"></div>
      </div>
      <div class="br-usage-card-body" id="body">
        <div class="br-usage-panels" id="panels">${tiles}</div>
        <div class="br-usage-card-heatmap">${heatmap}</div>
        <div class="br-usage-card-foot text-supporting" id="foot"><span>3 day streak</span><span>Less More</span></div>
      </div>
    </section>`;
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

describe('Home usage card in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the usage card geometry ' +
          'tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css =
      readFileSync(resolve(here, 'usage.css'), 'utf8') +
      heatmapCss() +
      segmentedTrackCss() +
      fillSurfaceCss();
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(
    width: number,
    { reducedMotion = 'reduce', html = cardHtml(width) } = {}
  ): Promise<Page> {
    const tab = await browser!.newPage({
      viewport: { width: 1000, height: 900 },
      reducedMotion: reducedMotion as 'reduce' | 'no-preference',
    });
    await tab.setContent(
      `<!doctype html><html><head><style>${TOKENS}\n${css}</style></head><body>` +
        `<div style="padding:20px">${html}</div></body></html>`
    );
    return tab;
  }

  it('is the chat measure, with a 36px head over a 1px hairline and the tabs on it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(760);
    try {
      const m = await tab.evaluate(() => {
        const box = (id: string) => document.getElementById(id)!.getBoundingClientRect();
        const head = document.getElementById('head')!;
        return {
          card: box('card').width,
          head: box('head').height,
          hairline: getComputedStyle(head).borderBottomWidth,
          tablistBottom: box('tablist').bottom,
          headContentBottom: box('head').bottom - 1,
          rangeCentre: box('range').top + box('range').height / 2,
          headCentre: box('head').top + (box('head').height - 1) / 2,
        };
      });
      expect(m.card).toBe(760);
      expect(m.head).toBe(37);
      expect(m.hairline).toBe('1px');
      // The tab underline (`-bottom-px`) lands on the hairline.
      expect(m.tablistBottom).toBe(m.headContentBottom);
      expect(Math.abs(m.rangeCentre - m.headCentre)).toBeLessThanOrEqual(0.5);
    } finally {
      await tab.close();
    }
  });

  it('is a fill with no outline, and the range track rests one ground deeper than it', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(760);
      try {
        const m = await tab.evaluate((dark) => {
          document.documentElement.classList.toggle('dark', dark);
          const style = (id: string) => getComputedStyle(document.getElementById(id)!);
          return {
            fill: style('card').backgroundColor,
            edgeWidth: style('card').borderTopWidth,
            edgeColor: style('card').borderTopColor,
            radius: style('card').borderTopLeftRadius,
            track: style('range').backgroundColor,
            trackHeight: document.getElementById('range')!.getBoundingClientRect().height,
          };
        }, mode === 'dark');
        expect(m.fill).toBe(mode === 'dark' ? 'rgb(35, 35, 32)' : 'rgb(244, 244, 242)');
        // The 1px edge stays, transparent, so forced colours draw it and nothing moves.
        expect(m.edgeWidth).toBe('1px');
        expect(m.edgeColor).toBe('rgba(0, 0, 0, 0)');
        expect(m.radius).toBe('12px');
        // A fill on the fill would be invisible: the track is the strong step.
        expect(m.track).toBe(mode === 'dark' ? 'rgb(44, 44, 41)' : 'rgb(234, 234, 230)');
        expect(m.track).not.toBe(m.fill);
        expect(m.trackHeight).toBe(28);
      } finally {
        await tab.close();
      }
    }
  });

  it('owes the deeper track to the fill-surface opt-in: without it the track IS the card', async (ctx) => {
    if (!browser) return ctx.skip();
    // The instrument's own proof: the same card with the class left off lays the track on the fill
    // (the invisible fill-on-fill the rule exists to prevent), so the assertion above can fail.
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(760, { html: cardHtml(760, { surface: false }) });
      try {
        const m = await tab.evaluate((dark) => {
          document.documentElement.classList.toggle('dark', dark);
          const style = (id: string) => getComputedStyle(document.getElementById(id)!);
          return { fill: style('card').backgroundColor, track: style('range').backgroundColor };
        }, mode === 'dark');
        expect(m.track).toBe(m.fill);
      } finally {
        await tab.close();
      }
    }
  });

  it('paints an empty day a step off the fill: white in light, the dark ramp\u2019s own step in dark', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(760);
      try {
        const m = await tab.evaluate((dark) => {
          document.documentElement.classList.toggle('dark', dark);
          const cell = document.querySelector('[role="gridcell"]')!;
          return {
            empty: getComputedStyle(cell).backgroundColor,
            card: getComputedStyle(document.getElementById('card')!).backgroundColor,
          };
        }, mode === 'dark');
        expect(m.empty).toBe(mode === 'dark' ? 'rgb(50, 49, 46)' : 'rgb(255, 255, 255)');
        expect(m.empty).not.toBe(m.card);
      } finally {
        await tab.close();
      }
    }
  });

  it('lays six flat tiles in three columns at 24px, and two under a 480px card', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const [width, columns] of [
      [760, 3],
      [470, 2],
    ] as const) {
      const tab = await open(width);
      try {
        const m = await tab.evaluate(() => {
          const tiles = [...document.querySelectorAll('.br-stat')].map((tile) =>
            tile.getBoundingClientRect()
          );
          const figure = document.querySelector('.br-stat-value')!;
          return {
            lefts: [...new Set(tiles.map((tile) => Math.round(tile.left)))].length,
            fontSize: getComputedStyle(figure).fontSize,
            panels: document.getElementById('panels')!.getBoundingClientRect().height,
          };
        });
        expect(m.lefts).toBe(columns);
        expect(m.fontSize).toBe('24px');
        expect(m.panels).toBeGreaterThanOrEqual(112);
      } finally {
        await tab.close();
      }
    }
  });

  it('draws a fluid heatmap: equal square columns, a 3px gap, the full inner width', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(760);
    try {
      const m = await tab.evaluate(() => {
        const rows = [...document.querySelectorAll('[role="row"]')];
        const first = [...rows[0].querySelectorAll('[role="gridcell"]')].map((cell) =>
          cell.getBoundingClientRect()
        );
        const grid = document.querySelector('[role="grid"]')!.getBoundingClientRect();
        const body = document.getElementById('body')!;
        const inner =
          body.getBoundingClientRect().width -
          parseFloat(getComputedStyle(body).paddingLeft) -
          parseFloat(getComputedStyle(body).paddingRight);
        const months = document.querySelector('.biorouter-heatmap-months')!;
        return {
          weeks: first.length,
          widths: first.map((cell) => cell.width),
          heights: first.map((cell) => cell.height),
          gap: first[1].left - first[0].right,
          gridWidth: grid.width,
          gridHeight: grid.height,
          inner,
          monthFont: getComputedStyle(months).fontSize,
          rowStarts: rows.map(
            (row) => row.querySelector('[role="gridcell"]')!.getBoundingClientRect().left
          ),
        };
      });
      // 2025-10-04 padded back to Sunday: 54 weeks in the window.
      expect(m.weeks).toBe(54);
      const width = m.widths[0];
      for (const w of m.widths) expect(Math.abs(w - width)).toBeLessThan(0.01);
      for (const h of m.heights) expect(Math.abs(h - width)).toBeLessThan(0.01);
      expect(m.gap).toBeCloseTo(3, 1);
      expect(m.inner).toBe(726);
      expect(m.gridWidth).toBeCloseTo(726, 0);
      // (726 − 53 × 3) / 54 ≈ 10.5px cells; seven rows and six gaps.
      expect(width).toBeCloseTo((726 - 53 * 3) / 54, 1);
      expect(m.gridHeight).toBeCloseTo(7 * width + 6 * 3, 0);
      // Every weekday row starts on the same column edge.
      expect(new Set(m.rowStarts.map((x) => x.toFixed(2))).size).toBe(1);
      expect(m.monthFont).toBe('12px');
    } finally {
      await tab.close();
    }
  });

  it('keeps every month label in one row, the last one flush with the grid edge, not past it', async (ctx) => {
    if (!browser) return ctx.skip();
    // The window ends on 2026-10-09; its last column starts on Sunday 2026-10-04, so its month
    // label lands in the final column, which is about 11px wide for a label about 21px wide.
    for (const width of [760, 560]) {
      const tab = await open(width);
      try {
        const m = await tab.evaluate(() => {
          const grid = document.querySelector('[role="grid"]')!.getBoundingClientRect();
          const labels = [...document.querySelectorAll('.biorouter-heatmap-months > span')].map(
            (label) => {
              const box = label.getBoundingClientRect();
              return {
                text: label.textContent,
                end: label.hasAttribute('data-end'),
                left: box.left,
                right: box.right,
                top: box.top,
              };
            }
          );
          return { gridRight: grid.right, labels };
        });
        const last = m.labels[m.labels.length - 1];
        expect(last.text).toBe('Oct');
        expect(last.end).toBe(true);
        expect(Math.abs(last.right - m.gridRight)).toBeLessThanOrEqual(0.5);
        // One row: a label that spans the whole row must not push the others down.
        expect(new Set(m.labels.map((label) => label.top)).size).toBe(1);
        // Never on top of the label before it.
        const before = m.labels[m.labels.length - 2];
        expect(last.left).toBeGreaterThan(before.right);
        // Only a label this close to the edge is flagged.
        expect(m.labels.filter((label) => label.end)).toHaveLength(1);
      } finally {
        await tab.close();
      }
    }
  });

  it('stacks the tab panels in one cell, so a crossfade never moves the card', async (ctx) => {
    if (!browser) return ctx.skip();
    const stack =
      `<section class="br-usage-card" style="width:760px"><div class="br-usage-card-body">` +
      `<div class="br-usage-panels" id="stack">` +
      `<div class="br-usage-panel" id="incoming" data-state="active" data-animate>` +
      renderToStaticMarkup(<StatGrid items={TILES} />) +
      `</div>` +
      `<div class="br-usage-panel" id="outgoing" data-state="inactive">` +
      `<ul class="br-usage-models"><li class="br-usage-model">One model</li></ul>` +
      `</div></div></div></section>`;
    for (const [reducedMotion, fade] of [
      ['no-preference', { incoming: 'br-fade-in', outgoing: 'br-fade-out' }],
      ['reduce', { incoming: 'none', outgoing: 'none' }],
    ] as const) {
      const tab = await open(760, { reducedMotion, html: stack });
      try {
        const m = await tab.evaluate(() => {
          const box = (id: string) => document.getElementById(id)!.getBoundingClientRect();
          const name = (id: string) => getComputedStyle(document.getElementById(id)!).animationName;
          return {
            incoming: box('incoming'),
            outgoing: box('outgoing'),
            stack: box('stack').height,
            incomingAnimation: name('incoming'),
            outgoingAnimation: name('outgoing'),
          };
        });
        // Same cell: the same box, so the heatmap below never jumps mid-fade.
        expect(m.outgoing.left).toBe(m.incoming.left);
        expect(m.outgoing.top).toBe(m.incoming.top);
        expect(m.outgoing.width).toBe(m.incoming.width);
        expect(m.stack).toBeGreaterThanOrEqual(112);
        expect(m.incomingAnimation).toBe(fade.incoming);
        expect(m.outgoingAnimation).toBe(fade.outgoing);
      } finally {
        await tab.close();
      }
    }
  });
});
