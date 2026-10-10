// @vitest-environment node
/**
 * Rung 0, measured in a real layout engine.
 *
 * jsdom lays nothing out, evaluates no grid and resolves no custom property, so
 * `styles/summaryRail.test.ts` can only hold the rail's rules at the source. What
 * they are FOR is geometry: at every seam the ladder decides, the conversation
 * must keep its full 760px measure, the rail must take exactly the column the
 * ladder gave it, and the composer's left edge must stay on the transcript
 * column's left edge.
 *
 * The real `summaryRail.css` and rung 2's block of `main.css` are loaded into
 * Chromium over markup shaped like BaseChat's split box (the area markers, the
 * transcript's `px-1` root and `paddingX={6}` viewport, the composer bar's 24px
 * gutters), with the app's Tailwind classes standing in as layered utilities.
 * Each case asks `summaryRailFit` for its answer and stamps it the way
 * `useSummaryRail` does, so the decision and the stylesheet are tested together.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { CHAT_MEASURE, summaryRailFit, type SummaryRailFit } from '../Layout/yieldLadder';

const here = dirname(fileURLToPath(import.meta.url));
const RAIL_CSS = readFileSync(resolve(here, 'summaryRail.css'), 'utf8');
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

/** Rung 2's grid (the preview split), exactly as `measures.test.ts` scopes it. */
function rungTwoBlock(): string {
  const start = MAIN_CSS.indexOf('[data-preview-split][data-preview-layout] {');
  const end = MAIN_CSS.indexOf(
    'body.biorouter-window-resizing .biorouter-sidebar-inset-depth',
    start
  );
  if (start < 0 || end < 0) throw new Error('rung 2 block not found in main.css');
  return MAIN_CSS.slice(start, end);
}

/** The app's Tailwind classes on these elements, as the layered utilities they are. */
const UTILITIES = `@layer utilities {
  .split { position: relative; display: flex; flex: 1 1 0%; min-height: 0; min-width: 0; }
  .column { display: flex; min-width: 0; flex: 1 1 0%; flex-direction: column; }
  .body { position: relative; display: flex; flex: 1 1 0%; min-height: 0; flex-direction: column; overflow: hidden; }
  .header { height: 44px; flex-shrink: 0; }
  .transcript { position: relative; flex: 1 1 0%; min-height: 0; padding-left: 4px; padding-right: 4px; }
  .viewport { height: 100%; padding-left: 24px; padding-right: 24px; }
  .chat-column { width: 100%; max-width: var(--measure-chat); margin-left: auto; margin-right: auto; height: 40px; }
  .composer-bar { flex-shrink: 0; padding: 28px 24px 24px; }
  .composer-shell { width: 100%; max-width: var(--measure-chat); margin-left: auto; margin-right: auto; height: 96px; }
}`;

const TOKENS = `:root {
  --measure-chat: ${CHAT_MEASURE}px;
  --chrome-height: 44px;
  --dock-height: 36px;
  --radius-container: 12px;
  --border-subtle: rgb(200, 200, 200);
  --background-default: rgb(255, 255, 255);
  --dur-fast: 125ms;
  --dur-fast-max: 175ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
body { margin: 0; font: 14px/20px sans-serif; }
* { box-sizing: border-box; }`;

const HEIGHT = 800;

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

type Layout = { pane: number; preview?: number; fit: SummaryRailFit };

function markup({ pane, preview, fit }: Layout): string {
  const split: string[] = ['data-preview-split=""', 'class="split"', 'id="split"'];
  const style: string[] = [`width: ${pane}px`, `height: ${HEIGHT}px`];
  if (preview) {
    split.push('data-preview-layout="side"');
    style.push(`--preview-panel-width: ${preview}px`);
  }
  if (fit.fits) {
    split.push('data-summary-rail=""');
    style.push(`--summary-rail-width: ${fit.width}px`);
  }
  return (
    `<div ${split.join(' ')} style="${style.join('; ')}">` +
    '<div data-preview-area="column" class="column">' +
    '<div data-preview-area="body" class="body">' +
    '<div data-preview-area="header" class="header" id="header"></div>' +
    '<div data-preview-area="transcript" data-preview-transcript="" class="transcript" id="transcript">' +
    '<div class="viewport"><div class="chat-column biorouter-chat-column" id="column"></div></div>' +
    '</div></div>' +
    '<div data-preview-area="composer" class="composer-bar" id="bar">' +
    '<div data-composer-shell="true" class="composer-shell" id="shell"></div></div>' +
    '</div>' +
    (fit.fits
      ? '<aside data-preview-area="rail" id="rail" data-rail-still="">' +
        '<div class="br-summary-card" data-state="open" data-entered="" id="card"><div style="height: 240px"></div></div>' +
        '</aside>'
      : '') +
    (preview ? '<div data-testid="artifact-viewer" id="preview"></div>' : '') +
    '</div>'
  );
}

type Box = { left: number; right: number; width: number; top: number };

async function boxes(page: Page): Promise<Record<string, Box | null>> {
  return page.evaluate(() => {
    const read = (id: string) => {
      const el = document.getElementById(id);
      if (!el) return null;
      const rect = el.getBoundingClientRect();
      return { left: rect.left, right: rect.right, width: rect.width, top: rect.top };
    };
    return {
      split: read('split'),
      header: read('header'),
      transcript: read('transcript'),
      column: read('column'),
      shell: read('shell'),
      rail: read('rail'),
      card: read('card'),
      preview: read('preview'),
    };
  });
}

describe('rung 0 in a real layout engine', () => {
  let browser: Browser | null = null;
  let page: Page | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      // Say so out loud: a silently skipped browser test reads as a passing one.
      console.warn(
        'No Chromium found — skipping the summary rail geometry tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
      return;
    }
    page = await browser.newPage({ viewport: { width: 2600, height: HEIGHT + 40 } });
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function lay(
    layout: Omit<Layout, 'fit'> & { previous?: SummaryRailFit | null; folded?: boolean }
  ) {
    // `folded`: the rail fits but is not shown (closed by the person, or no turn yet).
    const decided = summaryRailFit({
      paneWidth: layout.pane,
      previewMode: layout.preview ? 'side' : null,
      previewWidth: layout.preview ?? 0,
      previous: layout.previous ?? null,
    });
    const fit = layout.folded ? { fits: false, width: 0, measured: true } : decided;
    await page!.setContent(
      '<!doctype html><html><head><style>' +
        TOKENS +
        UTILITIES +
        rungTwoBlock() +
        RAIL_CSS +
        '</style></head><body>' +
        markup({ ...layout, fit }) +
        '</body></html>'
    );
    return { fit, at: await boxes(page!) };
  }

  it.each([
    [1055, null, false],
    [1056, null, true],
    [1067, { fits: false, width: 0, measured: true }, false],
    [1068, { fits: false, width: 0, measured: true }, true],
  ] as const)('a %ipx pane (previous %o) docks the rail: %s', async (pane, previous, docked) => {
    if (!page) return;
    const { fit, at } = await lay({ pane, previous });
    expect(fit.fits).toBe(docked);
    const column = at.column!;
    const shell = at.shell!;
    // The conversation keeps its full measure on both sides of every seam.
    expect(column.width).toBe(CHAT_MEASURE);
    // The composer's left edge is the transcript column's.
    expect(Math.abs(shell.left - column.left)).toBeLessThanOrEqual(0.5);
    expect(shell.width).toBe(CHAT_MEASURE);
    if (!docked) {
      expect(at.rail).toBeNull();
      // Centred in the whole pane.
      expect(Math.abs(column.left - (pane - CHAT_MEASURE) / 2)).toBeLessThanOrEqual(0.5);
      return;
    }
    const rail = at.rail!;
    // The rail takes exactly the column the ladder gave it, on the pane's right.
    expect(rail.width).toBe(fit.width);
    expect(rail.right).toBe(pane);
    // The conversation is centred in what is left.
    const cell = pane - fit.width;
    expect(Math.abs(column.left - (cell - CHAT_MEASURE) / 2)).toBeLessThanOrEqual(0.5);
    expect(column.right).toBeLessThanOrEqual(rail.left);
    // The header still spans the pane, above the rail.
    expect(at.header!.width).toBe(pane);
    expect(rail.top).toBeGreaterThanOrEqual(44);
    // The card sits in the cell's 12px padding, with none on the transcript side.
    expect(at.card!.left).toBe(rail.left);
    expect(at.card!.width).toBe(fit.width - 12);
    expect(at.card!.top).toBe(rail.top + 12);
  });

  it('pushes the column left by half the rail, the glide’s exact distance', async () => {
    if (!page) return;
    const without = (await lay({ pane: 1152, folded: true })).at.column!.left;
    const docked = await lay({ pane: 1152 });
    expect(docked.fit).toEqual({ fits: true, width: 280, measured: true });
    expect(without - docked.at.column!.left).toBe(docked.fit.width / 2);
  });

  it('seats the rail between the conversation and a side preview on an ultrawide pane', async () => {
    if (!page) return;
    const { fit, at } = await lay({ pane: 2272, preview: 920 });
    expect(fit).toEqual({ fits: true, width: 280, measured: true });
    const preview = at.preview!;
    const rail = at.rail!;
    expect(preview.width).toBe(920);
    expect(preview.right).toBe(2272);
    expect(rail.width).toBe(280);
    expect(rail.right).toBe(preview.left);
    expect(at.column!.width).toBe(CHAT_MEASURE);
    expect(at.column!.right).toBeLessThanOrEqual(rail.left);
    expect(Math.abs(at.shell!.left - at.column!.left)).toBeLessThanOrEqual(0.5);
  });

  it('leaves a narrower side preview alone: the rail folds, rung 2 is unchanged', async () => {
    if (!page) return;
    const { fit, at } = await lay({ pane: 1632, preview: 783 });
    expect(fit.fits).toBe(false);
    expect(at.rail).toBeNull();
    expect(at.preview!.width).toBe(783);
    expect(at.preview!.right).toBe(1632);
  });
});
