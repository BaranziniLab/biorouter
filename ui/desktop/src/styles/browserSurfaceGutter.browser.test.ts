// @vitest-environment node
/**
 * The browser surface keeps the composer on the transcript column's edges (WS-SUMMARY-1).
 *
 * In a `biorouter serve` browser the body carries `biorouter-headless-browser` and main.css widens
 * page gutters and caps the chat column and the composer at `min(760px, 100vw - 32px)`. The
 * composer bar's side padding is set inline in BaseChat.tsx to half `CHAT_COLUMN_CHROME`, the
 * transcript's own chrome (scroll root `px-1` plus viewport `px-6`), so the two boxes share their
 * left and right edges. A `!important` gutter on the bar in this family once beat that inline
 * value, and below an 816px cell the composer sat up to 12px off the column. That is geometry, so
 * it is measured in Chromium: every top level rule in main.css that names the browser surface,
 * over markup shaped like BaseChat's chat cell, at phone, laptop and wide widths with and without
 * a preview beside the chat.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { CHAT_COLUMN_CHROME, CHAT_MEASURE } from '../components/Layout/yieldLadder';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, 'main.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');

/** Every top level rule or at-rule in main.css that mentions the browser surface's body class. */
function browserSurfaceRules(): string {
  const chunks: string[] = [];
  let depth = 0;
  let from = 0;
  for (let i = 0; i < MAIN_CSS.length; i += 1) {
    const ch = MAIN_CSS[i];
    if (ch === '{') depth += 1;
    else if (ch === '}') {
      depth -= 1;
      if (depth === 0) {
        chunks.push(MAIN_CSS.slice(from, i + 1));
        from = i + 1;
      }
    } else if (ch === ';' && depth === 0) {
      from = i + 1;
    }
  }
  return chunks.filter((chunk) => chunk.includes('body.biorouter-headless-browser')).join('\n');
}

const SURFACE_CSS = browserSurfaceRules();

/** BaseChat's Tailwind classes on these elements, as the layered utilities they are. */
const UTILITIES = `@layer utilities {
  .cell { display: flex; flex-direction: column; height: 600px; }
  .transcript { flex: 1 1 0%; padding-left: 4px; padding-right: 4px; }
  .viewport { padding-left: 24px; padding-right: 24px; }
  .chat-column { width: 100%; max-width: ${CHAT_MEASURE}px; margin: 0 auto; height: 40px; }
  .composer-bar { flex-shrink: 0; padding-top: 28px; padding-bottom: 24px; }
  .composer { width: 100%; max-width: ${CHAT_MEASURE}px; margin: 0 auto; height: 96px; }
}
body { margin: 0; }
* { box-sizing: border-box; }`;

const html = (cell: number) => `<!doctype html><html><head><style>${UTILITIES}
${SURFACE_CSS}</style></head><body class="biorouter-headless-browser">
<div class="cell" style="width: ${cell}px">
  <div class="transcript"><div class="viewport">
    <div id="column" class="biorouter-chat-column chat-column"></div>
  </div></div>
  <div class="biorouter-chat-composer-bar composer-bar" style="padding-inline: ${CHAT_COLUMN_CHROME / 2}px">
    <div id="composer" class="biorouter-chat-composer composer"></div>
  </div>
</div></body></html>`;

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

/** [viewport width, chat cell width]: no sidebar on a phone, a 288px sidebar above 767px. */
const CASES: [number, number][] = [
  [375, 375],
  [767, 767],
  [900, 612],
  [1000, 712],
  [1200, 432],
  [1400, 1112],
  [1600, 832],
  [1920, 1152],
];

describe('the browser surface keeps the composer on the transcript column', () => {
  let browser: Browser | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found, skipping the browser surface gutter tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
  });

  afterAll(async () => {
    await browser?.close();
  });

  it('reads the browser surface family from main.css', () => {
    expect(SURFACE_CSS).toContain('.biorouter-chat-composer {');
  });

  it.for(CASES)('viewport %ipx, chat cell %ipx', async ([viewport, cell], ctx) => {
    if (!browser) return ctx.skip();
    const page = await browser.newPage({ viewport: { width: viewport, height: 800 } });
    try {
      await page.setContent(html(cell));
      const box = await page.evaluate(() => {
        const edges = (id: string) => {
          const r = document.getElementById(id)!.getBoundingClientRect();
          return [r.left, r.right];
        };
        return { column: edges('column'), composer: edges('composer') };
      });
      expect(box.composer, 'composer left and right edges').toEqual(box.column);
      const width = box.column[1] - box.column[0];
      expect(width, 'the column keeps the surface cap').toBeLessThanOrEqual(viewport - 24);
      expect(width, 'the column keeps the transcript chrome').toBeLessThanOrEqual(
        cell - CHAT_COLUMN_CHROME
      );
    } finally {
      await page.close();
    }
  });
});
