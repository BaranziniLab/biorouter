// @vitest-environment node
/**
 * Where the details pane lands, measured in a real layout engine.
 *
 * jsdom evaluates no container query and no grid, so every component test runs the pane in push
 * mode and cannot see cover mode at all. Cover is the ordinary layout, not an edge case: a
 * 1120–1280px window with the app sidebar open leaves the channel + pane region under 800px
 * (ui-redesign-spec, "Widths, the pane and the yield ladder").
 *
 * The defect this file pins: the connection bar sat inside `.crew-channel-body`, and a covering
 * pane both covered that region (inset by the header's height) and hid it (`visibility: hidden`).
 * The bar is where an error renders when the surface that caused it is not on screen — Stop in the
 * pane's Access tab, the sidebar, a menu, the observer — so while the pane was open those errors
 * were rendered exactly once and reached nobody, by sight or by screen reader.
 *
 * The real `crew-app.css` and the area stylesheets are loaded into Chromium over the stage's
 * skeleton; `paneCover.test.tsx` holds the skeleton to the shape the real layout renders.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { SKELETON_NOTE_HEIGHT, stageSkeleton, type StageSkeletonOptions } from './stageSkeleton';

const crewDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const STYLESHEETS = ['crew-app.css', 'channel/channel.css', 'pane/pane.css', 'layout/layout.css'];

/** The one token the stage's geometry reads; the rest only paint. */
const CHROME_HEIGHT = 44;
const SIDEBAR_WIDTH = 240;
const PANE_WIDTH = 360;
const HEIGHT = 600;
/** Channel + pane regions either side of the 800px push/cover threshold. */
const COVER_REGION = 700;
const PUSH_REGION = 1000;

/**
 * Launch whichever Chromium build this machine has: Playwright's download (full or headless
 * shell), else an installed Google Chrome, driven headless with a throwaway profile.
 */
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

interface Box {
  top: number;
  bottom: number;
  left: number;
  right: number;
  width: number;
  height: number;
}

interface Measured {
  stage: Box;
  header: Box;
  bar: Box;
  body: Box;
  composer: Box;
  pane: Box;
  paneHeader: Box;
  barVisibility: string;
  barDisplay: string;
  bodyVisibility: string;
  /** What a click at the middle of the bar lands on: inside the bar, or something above it. */
  barHit: 'bar' | 'pane' | 'other' | 'none';
  /** What a click at the middle of the body's area lands on. */
  bodyHit: 'body' | 'pane' | 'other' | 'none';
}

/** Half a CSS pixel: bounding rects are fractional. */
const EPSILON = 0.5;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

describe('the details pane over the channel, in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      // Say so out loud: a silently skipped browser test reads as a passing one.
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome) — skipping the Crew pane cover ' +
          'geometry tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = STYLESHEETS.map((path) => readFileSync(resolve(crewDir, path), 'utf8')).join('\n');
  }, 120_000);

  // An explicit budget, matching the launch: closing Chromium has overrun the default 30 s hook
  // timeout on a loaded machine (artifactCdnAssets.browser).
  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  const measure = async (region: number, options: StageSkeletonOptions): Promise<Measured> => {
    const page: Page = await browser!.newPage({
      viewport: { width: SIDEBAR_WIDTH + region + 40, height: HEIGHT + 40 },
      reducedMotion: 'reduce',
    });
    try {
      await page.setContent(
        '<!doctype html><html><head><style>' +
          `:root { --chrome-height: ${CHROME_HEIGHT}px; } body { margin: 0; font: 14px/20px sans-serif; }` +
          css +
          '</style></head><body>' +
          `<div style="width: ${SIDEBAR_WIDTH + region}px; height: ${HEIGHT}px">` +
          '<div class="crew-app"><div class="crew-sidebar"></div><div class="crew-main">' +
          stageSkeleton(options) +
          '</div></div></div></body></html>'
      );
      return await page.evaluate(() => {
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
        const hit = <Own extends string>(x: number, y: number, own: Own) => {
          const target = document.elementFromPoint(x, y);
          if (!target) return 'none' as const;
          if (target.closest(`#${own}`)) return own;
          if (target.closest('#pane')) return 'pane' as const;
          return 'other' as const;
        };
        const bar = box('bar');
        const body = box('body');
        const barStyle = getComputedStyle(document.getElementById('bar')!);
        return {
          stage: box('stage'),
          header: box('header'),
          bar,
          body,
          composer: box('composer'),
          pane: box('pane'),
          paneHeader: box('pane-header'),
          barVisibility: barStyle.visibility,
          barDisplay: barStyle.display,
          bodyVisibility: getComputedStyle(document.getElementById('body')!).visibility,
          barHit: bar.height > 0 ? hit(bar.left + 24, bar.bottom - 8, 'bar') : ('none' as const),
          bodyHit: hit(body.left + body.width / 2, body.top + body.height / 2, 'body'),
        };
      });
    } finally {
      await page.close();
    }
  };

  it('keeps the connection bar visible, and clickable, above a covering pane', async (ctx) => {
    if (!browser) return ctx.skip();
    const m = await measure(COVER_REGION, { note: true, pane: 'open' });

    // Cover mode really is on, or this measures nothing: the pane spans the stage.
    near(m.pane.left, m.stage.left, 'pane left');
    near(m.pane.right, m.stage.right, 'pane right');
    near(m.paneHeader.height, 40, 'the covering pane header row');

    // The band, then the bar's row sized by its note, then the pane over the body — below both.
    near(m.header.top, m.stage.top, 'header top');
    near(m.header.height, CHROME_HEIGHT, 'header height');
    near(m.bar.top, m.header.bottom, 'bar top');
    expect(m.bar.height).toBeGreaterThanOrEqual(SKELETON_NOTE_HEIGHT);
    near(m.pane.top, m.bar.bottom, 'pane top');
    near(m.pane.bottom, m.stage.bottom, 'pane bottom');
    expect(m.barVisibility).toBe('visible');
    expect(m.barHit).toBe('bar');

    // What the pane covers is the body, hidden while unseen (out of the tab order, draft kept).
    near(m.body.top, m.bar.bottom, 'body top');
    near(m.body.bottom, m.stage.bottom, 'body bottom');
    expect(m.bodyVisibility).toBe('hidden');
    expect(m.bodyHit).toBe('pane');
  });

  it('starts a covering pane right under the channel header when the bar is empty', async (ctx) => {
    if (!browser) return ctx.skip();
    const m = await measure(COVER_REGION, { note: false, pane: 'open' });

    expect(m.barDisplay).toBe('none');
    near(m.pane.top, m.header.bottom, 'pane top');
    near(m.pane.top - m.stage.top, CHROME_HEIGHT, 'pane inset');
    near(m.pane.bottom, m.stage.bottom, 'pane bottom');
    near(m.body.top, m.header.bottom, 'body top');
  });

  it('shows the body, under the bar, when the pane is closed in a narrow region', async (ctx) => {
    if (!browser) return ctx.skip();
    const m = await measure(COVER_REGION, { note: true, pane: 'closed' });

    expect(m.bodyVisibility).toBe('visible');
    expect(m.bodyHit).toBe('body');
    expect(m.barHit).toBe('bar');
    near(m.body.top, m.bar.bottom, 'body top');
    near(m.composer.bottom, m.stage.bottom, 'composer at the bottom');
    near(m.body.right, m.stage.right, 'the channel takes the whole width');
  });

  it('pushes a full-height pane beside the channel, with the bar in the channel column', async (ctx) => {
    if (!browser) return ctx.skip();
    const m = await measure(PUSH_REGION, { note: true, pane: 'open' });

    // The pane's own 44px band meets the channel header's at one top edge.
    near(m.pane.top, m.stage.top, 'pane top');
    near(m.pane.bottom, m.stage.bottom, 'pane bottom');
    near(m.pane.width, PANE_WIDTH, 'pane width');
    near(m.pane.right, m.stage.right, 'pane right');
    near(m.paneHeader.height, CHROME_HEIGHT, 'pane header band');

    near(m.header.right, m.pane.left, 'header ends at the pane');
    near(m.bar.top, m.header.bottom, 'bar top');
    near(m.bar.right, m.pane.left, 'bar ends at the pane');
    near(m.body.top, m.bar.bottom, 'body top');
    near(m.body.bottom, m.stage.bottom, 'body bottom');
    expect(m.bodyVisibility).toBe('visible');
    expect(m.barHit).toBe('bar');
    expect(m.bodyHit).toBe('body');
  });
});
