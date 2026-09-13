// @vitest-environment node
/**
 * Where the search highlighter paints, measured in a real layout engine.
 *
 * jsdom cannot answer any of this: it has no layout, `Range.getClientRects`
 * returns nothing there, and `line-clamp` / `overflow` are strings it stores
 * and never applies. The defect this file pins was invisible to every jsdom
 * test for exactly that reason — on Settings → Skills a description is
 * `line-clamp-1` (clientHeight 16, scrollHeight 48), and the highlighter built a
 * mark for every match in the paragraph, so the matches on the two clamped-away
 * lines were painted over the path line and the row padding below it, and the
 * counter counted them.
 *
 * The real `searchHighlighter.ts` is transpiled and run in Chromium against
 * small pages, one per shape a match can be hidden or shown in. The same class
 * serves the chat transcript and History, so half of these are guards against
 * the fix hiding a match that IS visible.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import ts from 'typescript';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));

/** Launch whichever Chromium build this machine downloaded (full or headless shell). */
const launchChromium = async (): Promise<Browser | null> => {
  for (const options of [{}, { channel: 'chromium' }] as const) {
    try {
      return await chromium.launch(options);
    } catch {
      // try the next build
    }
  }
  return null;
};

/** `searchHighlighter.ts` as a classic script that sets `window.SearchHighlighter`. */
const highlighterScript = (): string => {
  const source = readFileSync(resolve(here, 'searchHighlighter.ts'), 'utf-8');
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  });
  return `(() => { const exports = {}; ${outputText}; window.SearchHighlighter = exports.SearchHighlighter; })();`;
};

const PAGE_STYLE = `
  body { margin: 0; font: 12px/16px sans-serif; }
  #host { position: relative; height: 240px; width: 420px; overflow-y: auto; }
  .clamp { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 1; overflow: hidden; width: 300px; }
`;

interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
}

interface Measured {
  /** What `highlight()` returned — the number SearchView puts in the counter. */
  count: number;
  /** Every painted mark, in viewport coordinates. */
  marks: Box[];
  /** The box of `#clip`, when the scenario has one. */
  clip: Box | null;
}

/** Half a CSS pixel: bounding rects are fractional, client sizes are rounded. */
const EPSILON = 0.5;

const expectInside = (mark: Box, box: Box) => {
  expect(mark.left).toBeGreaterThanOrEqual(box.left - EPSILON);
  expect(mark.right).toBeLessThanOrEqual(box.right + EPSILON);
  expect(mark.top).toBeGreaterThanOrEqual(box.top - EPSILON);
  expect(mark.bottom).toBeLessThanOrEqual(box.bottom + EPSILON);
};

interface PageHighlighter {
  highlight(term: string): HTMLElement[];
  setCurrentMatch(index: number, shouldScroll?: boolean): void;
}

describe('SearchHighlighter in a real layout engine', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      // Say so out loud: a silently skipped browser test reads as a passing one.
      console.warn(
        'No Playwright Chromium build found — skipping the search highlight geometry tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
    script = highlighterScript();
  }, 120_000);

  // An explicit budget, matching the launch: closing Chromium has overrun the
  // default 30 s hook timeout on a loaded machine (artifactCdnAssets.browser).
  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  const withPage = async (body: string, run: (page: Page) => Promise<void>) => {
    const page = await browser!.newPage({ viewport: { width: 800, height: 600 } });
    try {
      await page.setContent(
        `<!doctype html><html><head><style>${PAGE_STYLE}</style></head><body>` +
          `<div id="host" data-search-scroll-area><div id="content">${body}</div></div>` +
          `</body></html>`
      );
      await page.addScriptTag({ content: script });
      await run(page);
    } finally {
      await page.close();
    }
  };

  /** Highlight `term` over `#content` and read back what was counted and painted. */
  const search = (page: Page, term: string): Promise<Measured> =>
    page.evaluate((searchTerm) => {
      const box = (r: DOMRect) => ({
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
        width: r.width,
        height: r.height,
      });
      const w = window as unknown as {
        SearchHighlighter: new (el: HTMLElement) => PageHighlighter;
        __highlighter?: PageHighlighter;
      };
      const highlighter = new w.SearchHighlighter(document.getElementById('content')!);
      w.__highlighter = highlighter;
      const count = highlighter.highlight(searchTerm).length;
      const marks = [...document.querySelectorAll('.search-highlight')].map((mark) =>
        box(mark.getBoundingClientRect())
      );
      const clipEl = document.getElementById('clip');
      return { count, marks, clip: clipEl ? box(clipEl.getBoundingClientRect()) : null };
    }, term);

  /** Navigate to match `index` the way the search bar does, then re-read marks and clip. */
  const navigate = (page: Page, index: number) =>
    page.evaluate((i) => {
      const box = (r: DOMRect) => ({
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
        width: r.width,
        height: r.height,
      });
      const w = window as unknown as { __highlighter: PageHighlighter };
      w.__highlighter.setCurrentMatch(i, true);
      const host = document.getElementById('host')!;
      const clipEl = document.getElementById('clip');
      const current = document.querySelector('.search-highlight.current');
      return {
        marks: [...document.querySelectorAll('.search-highlight')].map((mark) =>
          box(mark.getBoundingClientRect())
        ),
        current: current ? box(current.getBoundingClientRect()) : null,
        host: box(host.getBoundingClientRect()),
        hostScrollTop: host.scrollTop,
        clip: clipEl ? box(clipEl.getBoundingClientRect()) : null,
        clipScrollLeft: clipEl?.scrollLeft ?? 0,
      };
    }, index);

  /// The shipped defect, in the shape Settings → Skills renders it.
  it('does not count or paint a match on a line a line-clamp hid', async (ctx) => {
    if (!browser) return ctx.skip();
    await withPage(
      `<p id="clip" class="clamp" style="margin:0">zebra on the first line<br>second line<br>zebra on the third line</p>
       <div>path/under/the/description</div>`,
      async (page) => {
        const measured = await search(page, 'zebra');
        const clip = measured.clip!;
        // The clamp is real here, or this test measures nothing.
        expect(clip.height).toBe(16);

        expect(measured.count).toBe(1);
        expect(measured.marks).toHaveLength(1);
        measured.marks.forEach((mark) => expectInside(mark, clip));
      }
    );
  });

  it('does not count a match an overflow-hidden ancestor cut off', async (ctx) => {
    if (!browser) return ctx.skip();
    await withPage(
      `<div id="clip" style="height:16px; overflow:hidden">
         <div>quokka one</div><div>quokka two</div><div>quokka three</div>
       </div>`,
      async (page) => {
        const measured = await search(page, 'quokka');
        expect(measured.count).toBe(1);
        expect(measured.marks).toHaveLength(1);
        measured.marks.forEach((mark) => expectInside(mark, measured.clip!));
      }
    );
  });

  /// A match the clip only partly covers is still a match: it counts, and the
  /// part the user can see is what gets painted.
  it('counts a partly visible match and paints only its visible part', async (ctx) => {
    if (!browser) return ctx.skip();
    await withPage(
      `<div id="clip" style="width:120px; white-space:nowrap; overflow:hidden">` +
        `<span style="display:inline-block; width:100px"></span>walrus` +
        `<span style="display:inline-block; width:300px"></span>walrus</div>`,
      async (page) => {
        const measured = await search(page, 'walrus');
        const clip = measured.clip!;
        // The first `walrus` starts 100px in, so the 120px box shows its first
        // ~20px; the second starts past the box and is gone.
        expect(measured.count).toBe(1);
        expect(measured.marks).toHaveLength(1);
        expectInside(measured.marks[0], clip);
        expect(measured.marks[0].width).toBeGreaterThan(5);
        expect(measured.marks[0].left).toBeCloseTo(clip.left + 100, 0);
      }
    );
  });

  it('counts a line cut below its middle, and not one cut above it', async (ctx) => {
    if (!browser) return ctx.skip();
    // 16px lines. A 28px box shows most of the second line; a 19px box shows a
    // sliver of it — the shape a glyph box overhanging its line box takes.
    await withPage(
      `<div id="clip" style="height:28px; overflow:hidden">ibex one<br>ibex two<br>ibex three</div>`,
      async (page) => {
        const measured = await search(page, 'ibex');
        expect(measured.count).toBe(2);
        measured.marks.forEach((mark) => expectInside(mark, measured.clip!));
      }
    );
    await withPage(
      `<div id="clip" style="height:19px; overflow:hidden">lynx one<br>lynx two</div>`,
      async (page) => {
        const measured = await search(page, 'lynx');
        expect(measured.count).toBe(1);
        measured.marks.forEach((mark) => expectInside(mark, measured.clip!));
      }
    );
  });

  /// The transcript's own scroll container is not a clip. A match below the
  /// fold is one scroll away, counts, and navigation brings it into view.
  it('keeps counting a match the transcript has scrolled out of view', async (ctx) => {
    if (!browser) return ctx.skip();
    await withPage(
      `<div>okapi at the top</div><div style="height:900px"></div><div>okapi far below</div>`,
      async (page) => {
        const measured = await search(page, 'okapi');
        expect(measured.count).toBe(2);
        expect(measured.marks).toHaveLength(2);

        const after = await navigate(page, 1);
        expect(after.hostScrollTop).toBeGreaterThan(0);
        expect(after.current).not.toBeNull();
        expect(after.current!.top).toBeGreaterThanOrEqual(after.host.top);
        expect(after.current!.bottom).toBeLessThanOrEqual(after.host.bottom);
        // Scrolling the transcript neither drops nor adds a match.
        expect(after.marks).toHaveLength(2);
      }
    );
  });

  /// A code block that scrolls sideways is a scroll container, not a clip: its
  /// hidden text is one scroll away. It counts, is never painted outside the
  /// block, and navigating to it scrolls the block to show it.
  it('counts a match in a scrolled-away code block, and reveals it on navigation', async (ctx) => {
    if (!browser) return ctx.skip();
    await withPage(
      `<pre id="clip" style="width:200px; overflow-x:auto; margin:0; font:12px/16px monospace">` +
        `<span style="display:inline-block; width:500px"></span>narwhal</pre>`,
      async (page) => {
        const measured = await search(page, 'narwhal');
        expect(measured.count).toBe(1);
        measured.marks.forEach((mark) => expectInside(mark, measured.clip!));

        const after = await navigate(page, 0);
        expect(after.clipScrollLeft).toBeGreaterThan(0);
        expect(after.marks).toHaveLength(1);
        expectInside(after.marks[0], after.clip!);
        expect(after.marks[0].width).toBeGreaterThan(20);
      }
    );
  });

  it('does not count text that is not rendered', async (ctx) => {
    if (!browser) return ctx.skip();
    await withPage(
      `<div style="display:none">tapir hidden</div>
       <div style="visibility:hidden">tapir invisible</div>
       <div>tapir shown</div>`,
      async (page) => {
        const measured = await search(page, 'tapir');
        expect(measured.count).toBe(1);
        expect(measured.marks).toHaveLength(1);
      }
    );
  });

  /// An absolutely positioned child escapes an overflow-hidden parent when its
  /// containing block is further up. Treating that parent as a clip would hide
  /// a match that is on screen.
  it('does not hide a match an overflow-hidden parent does not actually clip', async (ctx) => {
    if (!browser) return ctx.skip();
    await withPage(
      `<div style="position:relative; height:40px">
         <div style="overflow:hidden; height:0">
           <span style="position:absolute; top:0; left:0">gecko</span>
         </div>
       </div>`,
      async (page) => {
        const measured = await search(page, 'gecko');
        expect(measured.count).toBe(1);
        expect(measured.marks).toHaveLength(1);
        expect(measured.marks[0].height).toBeGreaterThan(8);
      }
    );
  });

  /// The painted mark sits exactly on its text — checked against the text's own
  /// rect, not against the arithmetic that produced it.
  it('paints a visible match exactly over its text', async (ctx) => {
    if (!browser) return ctx.skip();
    await withPage(
      `<div style="padding:30px 0 0 50px">find <b id="word">heron</b> here</div>`,
      async (page) => {
        const measured = await search(page, 'heron');
        const word = await page.evaluate(() => {
          const range = document.createRange();
          range.selectNodeContents(document.getElementById('word')!);
          const r = range.getBoundingClientRect();
          return { left: r.left, top: r.top, width: r.width };
        });
        expect(measured.marks).toHaveLength(1);
        expect(measured.marks[0].left).toBeCloseTo(word.left, 0);
        expect(measured.marks[0].top).toBeCloseTo(word.top, 0);
        expect(measured.marks[0].width).toBeCloseTo(word.width, 0);
      }
    );
  });
});
