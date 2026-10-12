// @vitest-environment node
/**
 * Find keeps the match it shows clear of its own bar, measured in a real layout
 * engine (vision V2-R5-4).
 *
 * The bar floats over the top of the transcript (`.br-find-overlay`, 8px under
 * the cell top, 16px in from its right edge). A scroller cannot scroll above its
 * own top, so a match in the first lines of a chat, which is where "kdps" sat in
 * "Build kdps R package" (84..100 behind a bar ending at 94), stayed under the
 * bar however the target was computed. jsdom lays nothing out, so this holds
 * the rules where they are decided: the real `searchHighlighter.ts` (transpiled
 * and run as a classic script, like `searchHighlighter.browser.test.ts`) and
 * the real `styles/search.css` positioning the bar, over markup shaped like
 * BaseChat's transcript cell.
 *
 * What it pins, each with the control that shows the instrument can see it:
 * - the first match under the bar ends up below it, 8px clear (control: without
 *   a clearance it stays under it, which is the finding);
 * - the room is made in a way nobody sees: the text is where it was the instant
 *   the inset lands, and opening the bar made none (lazy);
 * - a match the bar does not cover moves nothing; a centred one centres below
 *   the bar;
 * - closing gives the room back, unseen where the reader has scrolled past it
 *   and as a glide where they have not.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import ts from 'typescript';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const SEARCH_CSS = readFileSync(resolve(here, '../styles/search.css'), 'utf8');

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

/** `searchHighlighter.ts` as a classic script that sets `SearchHighlighter` and `FindClearance`. */
const highlighterScript = (): string => {
  const source = readFileSync(resolve(here, 'searchHighlighter.ts'), 'utf-8');
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  });
  return (
    `(() => { const exports = {}; ${outputText}; ` +
    `window.SearchHighlighter = exports.SearchHighlighter; ` +
    `window.FindClearance = exports.FindClearance; })();`
  );
};

const TOKENS = `:root {
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
* { box-sizing: border-box; }
body { margin: 0; padding-top: 44px; font: 14px/20px Arial, sans-serif; }
/* BaseChat's cell: 44px under the window top (the band), the bar 8px under it. */
.cell { position: relative; overflow: hidden; width: 1100px; height: 600px; }
.viewport { height: 100%; width: 100%; overflow-y: scroll; }
.wrap { padding: 0 24px; }
.column { max-width: 760px; margin: 0 auto; padding-top: 20px; }
.user { text-align: right; padding: 12px 16px; height: 60px; }
.left { text-align: left; }
.message { height: 120px; padding: 12px 16px; border-bottom: 1px solid #eee; }
.bar { width: 100%; height: 42px; border: 1px solid rgb(200, 200, 200);
  border-radius: 12px; background: #fff; }`;

const MIN_GAP = 8;

interface SceneOptions {
  /** `right`: the first message's match sits under the bar's span; `left`: clear of it. */
  firstAt?: 'right' | 'left';
  /** How many 120px messages follow the first one. */
  messages?: number;
  /** `none`: no bar in the cell at all. */
  bar?: boolean;
}

const scene = ({ firstAt = 'right', messages = 20, bar = true }: SceneOptions = {}) =>
  `<!doctype html><html><head><style>${TOKENS}</style><style>${SEARCH_CSS}</style></head><body>` +
  `<div class="cell" id="cell"><div class="viewport" id="viewport" data-radix-scroll-area-viewport="">` +
  `<div class="wrap"><div class="column"><div id="content" class="search-container">` +
  `<div class="user ${firstAt === 'left' ? 'left' : ''}" id="first">Build the R package at github.com/UCSD/kdps</div>` +
  Array.from(
    { length: messages },
    (_, i) =>
      `<div class="message" id="m${i}">Message ${i}${i === 9 ? ' mentions kdps here' : ''}</div>`
  ).join('') +
  `</div></div></div></div>` +
  (bar
    ? `<div class="br-find-overlay" data-search-ui="" data-state="open" id="overlay"><div class="bar" id="bar"></div></div>`
    : '') +
  `</div></body></html>`;

type Box = { top: number; bottom: number; left: number; right: number };

describe('find keeps the current match clear of its bar', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found, skipping the find clearance geometry tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
    script = highlighterScript();
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  const withScene = async (
    options: SceneOptions,
    run: (page: Page) => Promise<void>,
    reducedMotion: 'reduce' | 'no-preference' = 'no-preference'
  ) => {
    const page = await browser!.newPage({
      viewport: { width: 1100, height: 700 },
      reducedMotion,
    });
    try {
      await page.setContent(scene(options));
      await page.addScriptTag({ content: script });
      // The bar's entrance, over, so it is measured at rest.
      await page.waitForTimeout(250);
      await run(page);
    } finally {
      await page.close();
    }
  };

  /** Start a search for `kdps` the way `SearchView` does (a highlighter, the session's clearance). */
  const startSearch = (page: Page, { clearance = true } = {}) =>
    page.evaluate((withClearance) => {
      const w = window as unknown as {
        SearchHighlighter: new (
          el: HTMLElement,
          onChange?: (count: number) => void,
          clearance?: unknown
        ) => {
          highlight(term: string): HTMLElement[];
          setCurrentMatch(index: number, shouldScroll?: boolean): void;
        };
        FindClearance: new (bar: () => Element | null) => unknown;
        __h: unknown;
        __clearance: unknown;
      };
      const clearance = withClearance
        ? new w.FindClearance(() => document.getElementById('overlay'))
        : undefined;
      w.__clearance = clearance ?? null;
      const highlighter = new w.SearchHighlighter(
        document.getElementById('content')!,
        undefined,
        clearance
      );
      w.__h = highlighter;
      return highlighter.highlight('kdps').length;
    }, clearance);

  /**
   * Navigate to match `index` as the bar's arrows do, then wait until the
   * scroll position has held still for 150ms. Not `scrollend`: making room is an
   * instant scroll of its own, and its `scrollend` arrives while the smooth
   * scroll that follows is still going.
   */
  const goTo = async (page: Page, index: number) => {
    await page.evaluate(
      (i) =>
        new Promise<void>((resolveRun) => {
          const w = window as unknown as {
            __h: { setCurrentMatch(index: number, shouldScroll?: boolean): void };
          };
          const viewport = document.getElementById('viewport')!;
          w.__h.setCurrentMatch(i, true);
          const startedAt = performance.now();
          let last = viewport.scrollTop;
          let since = startedAt;
          const tick = () => {
            const now = performance.now();
            if (viewport.scrollTop !== last) {
              last = viewport.scrollTop;
              since = now;
            }
            if (now - since > 150 || now - startedAt > 1500) resolveRun();
            else requestAnimationFrame(tick);
          };
          requestAnimationFrame(tick);
        }),
      index
    );
  };

  const box = (page: Page, selector: string): Promise<Box> =>
    page.evaluate((sel) => {
      const r = document.querySelector(sel)!.getBoundingClientRect();
      return { top: r.top, bottom: r.bottom, left: r.left, right: r.right };
    }, selector);

  const viewportState = (page: Page) =>
    page.evaluate(() => {
      const viewport = document.getElementById('viewport')!;
      return {
        scrollTop: viewport.scrollTop,
        paddingTop: getComputedStyle(viewport).paddingTop,
        inlinePaddingTop: viewport.style.paddingTop,
      };
    });

  // The finding, exactly: the first match is in the first lines of the chat and
  // under the bar's span, the scroller is at its top.
  it('control: without a clearance the first match stays behind the bar', async (ctx) => {
    if (!browser) return ctx.skip();
    await withScene({}, async (page) => {
      expect(await startSearch(page, { clearance: false })).toBe(2);
      await goTo(page, 0);
      const bar = await box(page, '#bar');
      const mark = await box(page, '.search-highlight.current');
      // Behind the bar: it starts above the bar's lower edge. (The instrument.)
      expect(mark.top).toBeLessThan(bar.bottom);
      expect(mark.bottom).toBeGreaterThan(bar.top);
      expect((await viewportState(page)).scrollTop).toBe(0);
    });
  });

  it('puts the first match below the bar, with the gap, where it could not scroll', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const motion of ['no-preference', 'reduce'] as const) {
      await withScene(
        {},
        async (page) => {
          await startSearch(page);
          await goTo(page, 0);
          const bar = await box(page, '#bar');
          const mark = await box(page, '.search-highlight.current');
          expect(mark.top).toBeGreaterThanOrEqual(bar.bottom + MIN_GAP - 0.5);

          // The mark is still painted over its own text, not over the shifted one.
          const text = await page.evaluate(() => {
            const range = document.createRange();
            const node = document.getElementById('first')!.firstChild as Text;
            const at = node.data.indexOf('kdps');
            range.setStart(node, at);
            range.setEnd(node, at + 4);
            const r = range.getBoundingClientRect();
            return { top: r.top, bottom: r.bottom };
          });
          expect(Math.abs(mark.top - text.top)).toBeLessThan(1);
          expect(Math.abs(mark.bottom - text.bottom)).toBeLessThan(1);

          // The room was made above the content, only as much as it took: the
          // first line was 84..100, 8px clear of a bar ending at 94 is 102.
          const state = await viewportState(page);
          expect(state.scrollTop).toBe(0);
          const inset = parseFloat(state.paddingTop);
          expect(inset).toBeGreaterThan(0);
          expect(inset).toBeLessThan(50);
        },
        motion
      );
    }
  });

  // Making room is an instant scroll, and its `scrollend` is dispatched after
  // the smooth scroll that follows has been asked for. If the highlighter took
  // that first `scrollend` for the end of its own scroll it would start
  // re-measuring every mark on each frame of the glide it is still running.
  it('keeps ignoring its own scroll until the smooth scroll has landed', async (ctx) => {
    if (!browser) return ctx.skip();
    await withScene({}, async (page) => {
      await startSearch(page);
      const run = await page.evaluate(
        () =>
          new Promise<{ heldFrames: number; releasedAt: number | null; scrolled: boolean }>(
            (resolveRun) => {
              const w = window as unknown as {
                __h: {
                  setCurrentMatch(index: number, shouldScroll?: boolean): void;
                  isScrollingProgrammatically: boolean;
                };
              };
              const viewport = document.getElementById('viewport')!;
              w.__h.setCurrentMatch(0, true);
              let heldFrames = 0;
              let scrolled = false;
              const startedAt = performance.now();
              const tick = () => {
                if (viewport.scrollTop > 1) scrolled = true;
                if (!w.__h.isScrollingProgrammatically) {
                  resolveRun({ heldFrames, releasedAt: viewport.scrollTop, scrolled });
                  return;
                }
                heldFrames += 1;
                if (performance.now() - startedAt > 2000) {
                  resolveRun({ heldFrames, releasedAt: null, scrolled });
                  return;
                }
                requestAnimationFrame(tick);
              };
              requestAnimationFrame(tick);
            }
          )
      );
      // It scrolled down the room it made (so the smooth scroll really ran), and
      // the hold was let go only with the scroller at its landing.
      expect(run.scrolled).toBe(true);
      expect(run.releasedAt).not.toBeNull();
      expect(run.releasedAt!).toBeLessThanOrEqual(1);
      expect(run.heldFrames).toBeGreaterThan(3);
    });
  });

  it('makes no room until a match needs it: opening the bar moves nothing', async (ctx) => {
    if (!browser) return ctx.skip();
    await withScene({}, async (page) => {
      const before = await box(page, '#first');
      await startSearch(page);
      const after = await box(page, '#first');
      expect(after.top).toBe(before.top);
      expect((await viewportState(page)).inlinePaddingTop).toBe('');
    });
  });

  // The text is where it was in the frame the inset lands; the scroll that
  // follows is what moves it, and that is the usual smooth one.
  it('holds the text still when the room is made, wherever the reader is scrolled', async (ctx) => {
    if (!browser) return ctx.skip();
    await withScene({}, async (page) => {
      await startSearch(page);
      for (const scrollTop of [0, 130]) {
        const moved = await page.evaluate((top) => {
          const w = window as unknown as {
            __clearance: { makeRoom(scroller: Element, by: number, reference: Element): boolean };
          };
          const viewport = document.getElementById('viewport')!;
          viewport.scrollTop = top;
          const content = document.getElementById('content')!;
          const at = viewport.scrollTop;
          const before = content.getBoundingClientRect().top;
          const grew = w.__clearance.makeRoom(viewport, 26, content);
          return {
            grew,
            moved: content.getBoundingClientRect().top - before,
            scrolledBy: viewport.scrollTop - at,
          };
        }, scrollTop);
        expect(moved.grew).toBe(true);
        // Absorbed by the scroll position, at the very top too (the padding is
        // scrollable room above the content): nothing on screen moved.
        expect(Math.abs(moved.moved)).toBeLessThan(0.5);
        expect(moved.scrolledBy).toBeCloseTo(26, 0);
        await page.evaluate(() => {
          (window as unknown as { __clearance: { release(): void } }).__clearance.release();
        });
      }
    });
  });

  it('moves nothing for a match the bar does not cover', async (ctx) => {
    if (!browser) return ctx.skip();
    await withScene({ firstAt: 'left' }, async (page) => {
      await startSearch(page);
      const bar = await box(page, '#bar');
      const mark0 = await box(page, '.search-highlight.current');
      // The control: it IS in the bar's vertical band, just not under it.
      expect(mark0.top).toBeLessThan(bar.bottom);
      expect(mark0.right).toBeLessThan(bar.left);
      await goTo(page, 0);
      const state = await viewportState(page);
      expect(state.inlinePaddingTop).toBe('');
      expect(state.scrollTop).toBe(0);
    });
  });

  it('centres a match in the part of the scroller below the bar', async (ctx) => {
    if (!browser) return ctx.skip();
    await withScene({}, async (page) => {
      await startSearch(page);
      await goTo(page, 1);
      const bar = await box(page, '#bar');
      const cell = await box(page, '#cell');
      const mark = await box(page, '.search-highlight.current');
      const freeTop = bar.bottom + MIN_GAP;
      const centre = (mark.top + mark.bottom) / 2;
      expect(centre).toBeCloseTo((freeTop + cell.bottom) / 2, 0);
      expect(mark.top).toBeGreaterThanOrEqual(freeTop);
      // A match in the middle of a chat needs no room above it.
      expect((await viewportState(page)).inlinePaddingTop).toBe('');
    });
  });

  it('only grows the room while the bar stays, and steps back to the first match cleanly', async (ctx) => {
    if (!browser) return ctx.skip();
    await withScene({}, async (page) => {
      await startSearch(page);
      await goTo(page, 0);
      const inset = parseFloat((await viewportState(page)).paddingTop);
      await goTo(page, 1);
      expect(parseFloat((await viewportState(page)).paddingTop)).toBe(inset);
      await goTo(page, 0);
      const bar = await box(page, '#bar');
      const mark = await box(page, '.search-highlight.current');
      expect(mark.top).toBeGreaterThanOrEqual(bar.bottom + MIN_GAP - 0.5);
      expect(parseFloat((await viewportState(page)).paddingTop)).toBe(inset);
    });
  });

  it('gives the room back unseen when the reader has scrolled past it', async (ctx) => {
    if (!browser) return ctx.skip();
    await withScene({}, async (page) => {
      await startSearch(page);
      await goTo(page, 0);
      const inset = parseFloat((await viewportState(page)).paddingTop);
      expect(inset).toBeGreaterThan(0);

      const run = await page.evaluate(() => {
        const w = window as unknown as { __clearance: { release(): void } };
        const viewport = document.getElementById('viewport')!;
        viewport.scrollTop = 500;
        const probe = document.getElementById('m5')!;
        const before = probe.getBoundingClientRect().top;
        w.__clearance.release();
        return {
          moved: probe.getBoundingClientRect().top - before,
          inline: viewport.style.paddingTop,
          padding: getComputedStyle(viewport).paddingTop,
        };
      });
      expect(Math.abs(run.moved)).toBeLessThan(0.5);
      expect(run.inline).toBe('');
      expect(run.padding).toBe('0px');
    });
  });

  it('gives the room back as a glide when the text sits below its resting place', async (ctx) => {
    if (!browser) return ctx.skip();
    await withScene({}, async (page) => {
      await startSearch(page);
      await goTo(page, 0);
      const resting = (await box(page, '#cell')).top + 20;

      const frames = await page.evaluate(
        () =>
          new Promise<number[]>((resolveRun) => {
            const w = window as unknown as { __clearance: { release(): void } };
            const first = document.getElementById('first')!;
            const tops: number[] = [first.getBoundingClientRect().top];
            w.__clearance.release();
            tops.push(first.getBoundingClientRect().top);
            const startedAt = performance.now();
            const tick = () => {
              tops.push(first.getBoundingClientRect().top);
              if (performance.now() - startedAt < 400) requestAnimationFrame(tick);
              else resolveRun(tops);
            };
            requestAnimationFrame(tick);
          })
      );

      // The first frame after the release is where the text was (no jump)...
      expect(Math.abs(frames[1] - frames[0])).toBeLessThan(0.5);
      // ...it ends in its resting place, 20px under the cell top...
      expect(frames[frames.length - 1]).toBeCloseTo(resting, 0);
      // ...and the way there is a glide: every step up, none of them the whole way.
      const steps = frames.slice(1);
      for (let i = 1; i < steps.length; i += 1) expect(steps[i]).toBeLessThanOrEqual(steps[i - 1]);
      expect(steps.some((top) => top < steps[0] - 0.5 && top > resting + 0.5)).toBe(true);

      const state = await viewportState(page);
      expect(state.inlinePaddingTop).toBe('');
      expect(state.paddingTop).toBe('0px');
      expect(await page.evaluate(() => document.getElementById('viewport')!.style.transition)).toBe(
        ''
      );
    });
  });

  it('jumps to the resting place under reduced motion', async (ctx) => {
    if (!browser) return ctx.skip();
    await withScene(
      {},
      async (page) => {
        await startSearch(page);
        await goTo(page, 0);
        const resting = (await box(page, '#cell')).top + 20;
        await page.evaluate(() => {
          const w = window as unknown as { __clearance: { release(): void } };
          w.__clearance.release();
        });
        expect((await box(page, '#first')).top).toBeCloseTo(resting, 0);
        expect((await viewportState(page)).inlinePaddingTop).toBe('');
      },
      'reduce'
    );
  });

  it('makes the room in a chat shorter than its viewport, where nothing can scroll', async (ctx) => {
    if (!browser) return ctx.skip();
    await withScene({ messages: 2 }, async (page) => {
      await startSearch(page);
      const room = await page.evaluate(() => {
        const viewport = document.getElementById('viewport')!;
        return viewport.scrollHeight - viewport.clientHeight;
      });
      expect(room).toBeLessThanOrEqual(0);

      await goTo(page, 0);
      const bar = await box(page, '#bar');
      const mark = await box(page, '.search-highlight.current');
      expect(mark.top).toBeGreaterThanOrEqual(bar.bottom + MIN_GAP - 0.5);
      expect(parseFloat((await viewportState(page)).paddingTop)).toBeGreaterThan(0);

      // And closing puts it back where it was, still short of overflow.
      const resting = (await box(page, '#cell')).top + 20;
      await page.evaluate(() => {
        const w = window as unknown as { __clearance: { release(): void } };
        w.__clearance.release();
      });
      await page.waitForTimeout(400);
      expect((await box(page, '#first')).top).toBeCloseTo(resting, 0);
      expect((await viewportState(page)).inlinePaddingTop).toBe('');
    });
  });

  it('is a no-op when there is no bar over the transcript', async (ctx) => {
    if (!browser) return ctx.skip();
    await withScene({ bar: false }, async (page) => {
      await startSearch(page);
      const before = await box(page, '#first');
      await goTo(page, 0);
      expect((await box(page, '#first')).top).toBe(before.top);
      expect((await viewportState(page)).inlinePaddingTop).toBe('');
      const barBox = await page.evaluate(() => {
        const w = window as unknown as {
          __clearance: { barBox(scroller: Element): unknown } | null;
        };
        return w.__clearance?.barBox(document.getElementById('viewport')!) ?? null;
      });
      expect(barBox).toBeNull();
    });
  });
});
