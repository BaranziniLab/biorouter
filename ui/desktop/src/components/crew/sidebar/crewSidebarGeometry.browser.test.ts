// @vitest-environment node
/**
 * The Crew column and the switcher band, measured in a real layout engine (Q2-39, Q2-40).
 *
 * jsdom lays nothing out, evaluates no grid and no container query, so the component tests can
 * only hold these rules at the source. What they are FOR is geometry, and round 1's fix for the
 * titlebar reserve shows why that is not enough: widening the column by the reserve (T-21) read
 * fine as CSS and left the channel 652px at a 1048px window, under the 800px the details pane
 * needs to push, so the pane covered the conversation.
 *
 * The real `crew-app.css` and `sidebar/crew-sidebar.css` (and the stage's area stylesheets) are
 * loaded into Chromium over markup shaped like the app: the app sidebar's own `data-slot` peers,
 * the Crew sidebar's band, switcher and status row, and the stage skeleton the pane tests use.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { stageSkeleton } from '../integration/stageSkeleton';

const crewDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const STYLESHEETS = [
  'crew-app.css',
  'sidebar/crew-sidebar.css',
  'channel/channel.css',
  'pane/pane.css',
  'layout/layout.css',
];

/** The tokens the geometry reads, at `main.css`'s values; the rest only paint. */
const TOKENS = `:root {
  --chrome-height: 44px;
  --control-md: 32px;
  --row-height-rail: 32px;
  --icon-row: 16px;
  --radius-element: 8px;
  --radius-inner: 4px;
  --text-label: 14px;
  --text-label--line-height: 20px;
  --text-supporting: 12px;
  --text-supporting--line-height: 16px;
  --text-chip: 12px;
  --text-chip--line-height: 16px;
  --biorouter-titlebar-control-reserve: 172px;
  --border-subtle: rgb(200, 200, 200);
  --sidebar-border: rgb(200, 200, 200);
  --dur-fast: 120ms;
  --dur-fast-min: 80ms;
  --dur-fast-max: 175ms;
  --dur-med: 300ms;
  --dur-slow: 525ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
body { margin: 0; font: 14px/20px sans-serif; }
* { box-sizing: border-box; }`;

/** The default window, and the app sidebar's width when it is open. */
const WINDOW = { width: 1048, height: 760 };
const APP_SIDEBAR = 288;
const CREW_COLUMN = 240;
const PANE_WIDTH = 360;
const PUSH_MIN = 800;

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

/** The page: the app sidebar (collapsed: 0px; open: 288px) beside its inset, which holds Crew. */
function page(css: string, appSidebar: 'collapsed' | 'expanded', pane: 'open' | 'closed') {
  const sidebarWidth = appSidebar === 'collapsed' ? 0 : APP_SIDEBAR;
  return (
    '<!doctype html><html><head><style>' +
    TOKENS +
    css +
    '</style></head><body>' +
    `<div style="display: flex; width: ${WINDOW.width}px; height: ${WINDOW.height}px">` +
    `<div data-slot="sidebar" data-state="${appSidebar}" style="flex: none; width: ${sidebarWidth}px"></div>` +
    '<div data-slot="sidebar-inset" style="flex: 1 1 auto; min-width: 0; height: 100%">' +
    '<div class="crew-app" id="app"><div class="crew-sidebar" id="column">' +
    '<nav class="crew-sidebar-nav">' +
    '<div class="crew-sidebar-band" id="band">' +
    '<button type="button" class="crew-sidebar-switcher" id="switcher">' +
    '<span class="crew-sidebar-switcher-name" id="name">chen-lab</span>' +
    '<svg class="crew-sidebar-chevron" viewBox="0 0 16 16"></svg>' +
    '</button></div>' +
    '<div class="crew-sidebar-status" id="status"><div class="crew-sidebar-status-word">Connected</div></div>' +
    '</nav></div>' +
    '<div class="crew-main" id="main">' +
    stageSkeleton({ note: false, pane }) +
    '</div></div></div></div></body></html>'
  );
}

const EPSILON = 0.5;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

describe('the Crew column in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome) — skipping the Crew column geometry ' +
          'tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = STYLESHEETS.map((path) => readFileSync(resolve(crewDir, path), 'utf8')).join('\n');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(
    appSidebar: 'collapsed' | 'expanded',
    pane: 'open' | 'closed',
    reducedMotion: 'reduce' | 'no-preference' = 'reduce'
  ): Promise<Page> {
    const tab = await browser!.newPage({ viewport: WINDOW, reducedMotion });
    await tab.setContent(page(css, appSidebar, pane));
    return tab;
  }

  const measure = (tab: Page) =>
    tab.evaluate(() => {
      const box = (id: string) => {
        const r = document.getElementById(id)!.getBoundingClientRect();
        return {
          top: r.top,
          bottom: r.bottom,
          left: r.left,
          right: r.right,
          width: r.width,
          height: r.height,
        };
      };
      const name = document.getElementById('name')!;
      const band = getComputedStyle(document.getElementById('band')!);
      return {
        column: box('column'),
        band: box('band'),
        switcher: box('switcher'),
        status: box('status'),
        main: box('main'),
        body: box('body'),
        pane: box('pane'),
        header: box('header'),
        nameCut: name.scrollWidth > name.clientWidth,
        bandBorder: band.borderBottomWidth,
        bandLine: `${band.backgroundPositionY} ${band.backgroundSize}`,
        bandLineDrawn: band.backgroundImage.startsWith('linear-gradient'),
      };
    });

  it('keeps the column 240px and gives the channel room to push at 1048px, sidebar collapsed (Q2-39)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('collapsed', 'open');
    try {
      const m = await measure(tab);
      near(m.column.width, CREW_COLUMN, 'Crew column');
      near(m.main.width, WINDOW.width - CREW_COLUMN, 'channel + pane region');
      expect(m.main.width).toBeGreaterThanOrEqual(PUSH_MIN);
      // The pane PUSHES: beside the channel, full height, never over it.
      near(m.pane.width, PANE_WIDTH, 'pane width');
      near(m.pane.top, m.main.top, 'pane top');
      near(m.body.right, m.pane.left, 'channel ends where the pane starts');
    } finally {
      await tab.close();
    }
  });

  it('moves the switcher into its own row below the band, and shows "chen-lab" whole', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('collapsed', 'closed');
    try {
      const m = await measure(tab);
      // The band keeps the 44px titlebar reserve and nothing of Crew's under the controls…
      near(m.band.top, 0, 'band top');
      near(m.band.height, 44 + 36, 'band plus the switcher row');
      expect(m.switcher.top).toBeGreaterThanOrEqual(44);
      // …the switcher is centred in the 36px row under it, at the band's own inset, not pushed
      // right by a margin…
      near(m.switcher.top + m.switcher.height / 2, 44 + 18, 'switcher centre');
      near(m.switcher.left, m.column.left + 8, 'switcher left');
      // …the name is not cut…
      expect(m.nameCut).toBe(false);
      // …the status row follows the switcher's row…
      near(m.status.top, m.band.bottom, 'status row top');
      // …and the band's hairline stays at y=44, the channel header's bottom edge.
      expect(m.bandBorder).toBe('0px');
      expect(m.bandLine).toBe('43px 100% 1px');
      expect(m.bandLineDrawn).toBe(true);
      near(m.header.bottom, 44, 'channel header bottom');
    } finally {
      await tab.close();
    }
  });

  it('keeps the switcher in the 44px band when the app sidebar is open', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('expanded', 'closed');
    try {
      const m = await measure(tab);
      near(m.column.width, CREW_COLUMN, 'Crew column');
      near(m.band.height, 44, 'band');
      expect(m.switcher.bottom).toBeLessThanOrEqual(44);
      near(m.switcher.left, m.column.left + 8, 'switcher left');
      expect(m.bandBorder).toBe('1px');
      near(m.status.top, 44, 'status row top');
    } finally {
      await tab.close();
    }
  });

  /**
   * Q2-40 read "the grid snaps while the aside slides". It does not: column 2 is `auto`, so it
   * takes the aside's animated width on every frame, and the channel narrows with it. This pins
   * that, so nobody adds a `grid-template-columns` transition that could never run.
   */
  it('narrows the channel with the pane, frame by frame, as the pane pushes in (Q2-40)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('collapsed', 'closed', 'no-preference');
    try {
      const samples = await tab.evaluate(async () => {
        const pane = document.getElementById('pane')!;
        const body = document.getElementById('body')!;
        const frame = () => new Promise((done) => requestAnimationFrame(done));
        await frame();
        await frame();
        pane.dataset.state = 'open';
        const out: { channel: number; pane: number }[] = [];
        const start = performance.now();
        while (performance.now() - start < 500) {
          await frame();
          out.push({
            channel: body.getBoundingClientRect().width,
            pane: pane.getBoundingClientRect().width,
          });
        }
        return out;
      });
      const region = WINDOW.width - CREW_COLUMN;
      // Every frame, the two share the region exactly: the grid follows the aside.
      for (const sample of samples) near(sample.channel + sample.pane, region, 'channel + pane');
      // And there ARE frames between: the channel passes through widths it would skip if it
      // snapped.
      const between = samples.filter((sample) => sample.pane > 20 && sample.pane < PANE_WIDTH - 20);
      expect(between.length).toBeGreaterThanOrEqual(2);
      near(samples[samples.length - 1].pane, PANE_WIDTH, 'pane at rest');
    } finally {
      await tab.close();
    }
  });
});
