// @vitest-environment node
/**
 * Find in chat, measured in a real layout engine (spec 3.16, 5.7).
 *
 * jsdom lays nothing out, so `styles/searchBarNote.test.ts` can only hold the
 * overlay's rules at the source. What they are FOR is geometry: opening the bar
 * must not move the conversation by a pixel, the bar must sit 8px under the top
 * of the transcript cell and 16px in from its right edge, it must stay put
 * while the conversation scrolls under it, and its two-row "type at least…"
 * state must show whole (the old `max-height` reveal clipped it).
 *
 * The real `styles/search.css` is loaded into Chromium over markup shaped like
 * BaseChat's transcript cell: a `relative` + `overflow: hidden` ScrollArea root,
 * a scrolling viewport and the 760px chat column. `SearchView` portals the bar
 * into the cell, after the viewport; the first cases also hold it inside the
 * column, where it used to render, to show what the portal is for.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const SEARCH_CSS = readFileSync(resolve(here, '../../styles/search.css'), 'utf8');

const TOKENS = `:root {
  --measure-chat: 760px;
  --dur-fast: 125ms;
  --dur-fast-max: 175ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
  --icon-row: 16px;
  --text-secondary: 13px;
  --text-secondary--line-height: 18px;
  --text-supporting: 12px;
  --text-supporting--line-height: 16px;
  --border-subtle: rgb(200, 200, 200);
  --background-default: rgb(255, 255, 255);
  --background-medium: rgb(235, 235, 235);
  --text-default: rgb(20, 20, 20);
  --text-muted: rgb(110, 110, 110);
  --font-sans: Arial, sans-serif;
}
@keyframes br-fade-in { from { opacity: 0; } to { opacity: 1; } }
@keyframes br-fade-out { from { opacity: 1; } to { opacity: 0; } }
body { margin: 0; font: 14px/20px Arial, sans-serif; }
* { box-sizing: border-box; }
.cell { position: relative; overflow: hidden; width: 1000px; height: 600px; }
.viewport { height: 100%; overflow-y: scroll; padding: 0 24px; }
.column { max-width: 760px; margin: 0 auto; }
.message { height: 120px; border-bottom: 1px solid #ddd; }
.bar { width: 100%; max-width: 760px; border: 1px solid rgb(200, 200, 200);
  border-radius: 12px; background: #fff; overflow: hidden; display: flex; flex-direction: column; }
.button { width: 28px; height: 28px; flex: none; }`;

/** The bar's markup as `SearchBar` renders it, without React. */
const bar = (note: boolean) =>
  `<div class="br-find-overlay" data-search-ui="" data-state="open" id="overlay">` +
  `<div class="bar br-find-bar" id="bar">` +
  `<div class="br-find-row"><svg class="br-find-icon"></svg>` +
  `<input class="br-find-input" placeholder="Find in chat…">` +
  `<span class="br-find-count">3 of 12</span>` +
  `<span class="button"></span><span class="button"></span><span class="button"></span><span class="button"></span></div>` +
  (note
    ? `<div class="br-find-note" id="note">Type at least 2 characters to search. ` +
      `A second line of help text that wraps in a narrow bar.</div>`
    : '') +
  `</div></div>`;

/**
 * `place`: `column` is where the bar used to render (inside the scrolled
 * content), `cell` is where `SearchView` portals it (the cell's last child).
 * `highlighting`: a search is running, so `SearchHighlighter` has set the
 * viewport to `position: relative` (it positions its marks against it).
 */
const transcript = (
  withBar: 'none' | 'bar' | 'note',
  {
    place = 'cell',
    highlighting = false,
  }: { place?: 'column' | 'cell'; highlighting?: boolean } = {}
) => {
  const theBar = withBar === 'none' ? '' : bar(withBar === 'note');
  const viewportStyle = highlighting ? ' style="position: relative"' : '';
  return (
    `<div class="cell" id="cell"><div class="viewport" id="viewport"${viewportStyle}>` +
    `<div class="column">` +
    (place === 'column' ? theBar : '') +
    Array.from({ length: 20 }, (_, i) => `<div class="message" id="m${i}">Message ${i}</div>`).join(
      ''
    ) +
    `</div></div>` +
    (place === 'cell' ? theBar : '') +
    `</div>`
  );
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

type Rect = { top: number; left: number; right: number; bottom: number; height: number };

const rectOf = (page: Page, id: string): Promise<Rect> =>
  page.evaluate((target) => {
    const r = document.getElementById(target)!.getBoundingClientRect();
    return { top: r.top, left: r.left, right: r.right, bottom: r.bottom, height: r.height };
  }, id);

describe('find in chat floats over the transcript', () => {
  let browser: Browser | null = null;
  let page: Page;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found — skipping the find overlay geometry tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
      return;
    }
    page = await browser.newPage({
      viewport: { width: 1100, height: 700 },
      reducedMotion: 'reduce',
    });
  });

  afterAll(async () => {
    await browser?.close();
  });

  const load = async (
    variant: 'none' | 'bar' | 'note',
    options?: Parameters<typeof transcript>[1]
  ) => {
    await page.setContent(
      `<!doctype html><html><head><style>${TOKENS}</style><style>${SEARCH_CSS}</style></head>` +
        `<body>${transcript(variant, options)}</body></html>`
    );
    // Let the (reduced) entrance finish so the bar is measured at rest.
    await page.waitForTimeout(50);
  };

  it('opening it moves the conversation by zero pixels', async () => {
    if (!browser) return;
    await load('none');
    const before = await rectOf(page, 'm0');
    await load('bar');
    const after = await rectOf(page, 'm0');
    expect(after.top).toBe(before.top);
    expect(after.left).toBe(before.left);
  });

  it('sits 8px under the cell top and 16px in from its right edge, 440px wide at most', async () => {
    if (!browser) return;
    await load('bar');
    const cell = await rectOf(page, 'cell');
    const overlay = await rectOf(page, 'overlay');
    expect(overlay.top - cell.top).toBe(8);
    expect(cell.right - overlay.right).toBe(16);
    expect(overlay.right - overlay.left).toBeLessThanOrEqual(440);
  });

  it('stays put while the conversation scrolls under it', async () => {
    if (!browser) return;
    await load('bar');
    const before = await rectOf(page, 'overlay');
    await page.evaluate(() => {
      document.getElementById('viewport')!.scrollTop = 600;
    });
    const after = await rectOf(page, 'overlay');
    expect(after.top).toBe(before.top);
    // …while the conversation itself did move.
    expect((await rectOf(page, 'm0')).top).toBeLessThan(0);
  });

  // Vision V2-02, the cause: once a search runs, the viewport is positioned,
  // and a bar inside the scrolled content scrolls away with the text. This is
  // the control that shows the instrument can see the bug.
  it('inside the scrolled content, a running search carries the bar off the screen', async () => {
    if (!browser) return;
    await load('bar', { place: 'column', highlighting: true });
    const before = await rectOf(page, 'overlay');
    await page.evaluate(() => {
      document.getElementById('viewport')!.scrollTop = 300;
    });
    expect((await rectOf(page, 'overlay')).top).toBe(before.top - 300);
  });

  it('portalled into the cell, it stays put while a running search scrolls the text', async () => {
    if (!browser) return;
    await load('bar', { place: 'cell', highlighting: true });
    const cell = await rectOf(page, 'cell');
    const before = await rectOf(page, 'overlay');
    expect(before.top - cell.top).toBe(8);
    expect(cell.right - before.right).toBe(16);
    await page.evaluate(() => {
      document.getElementById('viewport')!.scrollTop = 300;
    });
    expect((await rectOf(page, 'overlay')).top).toBe(before.top);
    expect((await rectOf(page, 'm0')).top).toBeLessThan(0);
  });

  it('shows the two-row note whole: the bar grows instead of clipping it', async () => {
    if (!browser) return;
    await load('note');
    const barRect = await rectOf(page, 'bar');
    const note = await rectOf(page, 'note');
    expect(note.height).toBeGreaterThan(20);
    expect(note.bottom).toBeLessThanOrEqual(barRect.bottom);
    const hiddenOverflow = await page.evaluate(() => {
      const el = document.getElementById('note')!;
      return el.scrollHeight - el.clientHeight;
    });
    expect(hiddenOverflow).toBe(0);
  });
});
