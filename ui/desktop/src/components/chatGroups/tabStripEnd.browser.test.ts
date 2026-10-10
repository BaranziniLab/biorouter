// @vitest-environment node
/**
 * The strip's "+" once its tabs overflow, measured in a real engine (vision
 * V1-M2, spec 5.7).
 *
 * jsdom lays nothing out, so `ChatTabStrip.test.tsx` can only pin where the "+"
 * is mounted. What that placement is FOR is a hit test: with nine tabs and the
 * last one revealed, the "+" at the end of the scroll content sat scrolled out
 * under the ▾ (3px of it showing), and a press on it landed on the ▾. The real
 * strip rules from `main.css` and `chat-tabs.css` run here in Chromium over the
 * strip's shape: the wrap, the scroll box, the tabs, the end slot and the ▾.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const CHAT_TABS_CSS = readFileSync(resolve(here, 'chat-tabs.css'), 'utf8');

/** One top-level block of main.css, from its selector line to its closing brace. */
function block(opening: string): string {
  const start = MAIN_CSS.indexOf(opening);
  if (start < 0) throw new Error(`main.css has no block starting ${opening}`);
  let depth = 0;
  for (let i = MAIN_CSS.indexOf('{', start); i < MAIN_CSS.length; i += 1) {
    if (MAIN_CSS[i] === '{') depth += 1;
    if (MAIN_CSS[i] === '}') depth -= 1;
    if (depth === 0) return MAIN_CSS.slice(start, i + 1);
  }
  throw new Error(`unbalanced block ${opening}`);
}

const STRIP_CSS = [
  '.br-tabstrip {',
  '.br-tabstrip__overflow {',
  '.br-tabstrip--inline {\n  background',
  '.br-tab {',
]
  .map(block)
  .join('\n');

const TOKENS = `:root {
  --tab-height: 32px;
  --tab-min-width: 136px;
  --radius-inner: 4px;
  --radius-md: 6px;
  --font-sans: Arial, sans-serif;
  --text-secondary: 13px;
  --dur-fast: 125ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
  --sidebar: #f5f5f5;
  --border-subtle: #ddd;
  --border-strong: #bbb;
  --background-default: #fff;
  --text-muted: #666;
}
body { margin: 0; font: 13px Arial, sans-serif; }
* { box-sizing: border-box; }
/* The wrap and the scroll box, as ChatTabStrip's utilities and inline styles
   lay them out (flex-1 min-w-0, the 44px drag gutter, no left padding). */
.br-tabstrip-wrap { display: flex; align-items: center; height: 44px; width: 640px; min-width: 0;
  padding-right: 44px; }
#strip { flex: 1 1 0%; min-width: 0; height: 100%; padding-left: 0; }
.slot { flex: none; }
.plus { display: flex; width: 24px; height: 24px; margin-left: 2px; border: 0; padding: 0; }`;

const TAB_COUNT = 9;

const strip = (plus: 'inside' | 'outside') => {
  const tabs = Array.from(
    { length: TAB_COUNT },
    (_, i) =>
      `<div class="br-tab" data-tab-id="t${i}" id="t${i}"><span>A fairly long chat title ${i}</span></div>`
  ).join('');
  const button = `<button class="plus" id="plus" type="button">+</button>`;
  return (
    `<div class="br-tabstrip-wrap" id="wrap">` +
    `<div class="br-tabstrip br-tabstrip--inline" id="strip">${tabs}` +
    (plus === 'inside' ? `<div class="slot">${button}</div>` : '') +
    `</div>` +
    (plus === 'outside' ? `<div class="br-tabstrip__end" id="end">${button}</div>` : '') +
    `<button class="br-tabstrip__overflow" id="more" type="button">v</button>` +
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

type Probe = {
  overflowing: boolean;
  plusHit: string | null;
  moreHit: string | null;
  plus: { left: number; right: number };
  more: { left: number; right: number };
};

describe('the "+" stays reachable once the tabs overflow', () => {
  let browser: Browser | null = null;
  let page: Page;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found — skipping the tab strip end-slot tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
      return;
    }
    page = await browser.newPage({ viewport: { width: 800, height: 200 } });
  });

  afterAll(async () => {
    await browser?.close();
  });

  /** Mount the strip, reveal the LAST tab as `revealActiveTab` does, and hit-test. */
  const probe = async (plus: 'inside' | 'outside'): Promise<Probe> => {
    await page.setContent(
      `<!doctype html><html><head><style>${TOKENS}</style><style>${STRIP_CSS}</style>` +
        `<style>${CHAT_TABS_CSS}</style></head><body>${strip(plus)}</body></html>`
    );
    return page.evaluate(
      (lastId) => {
        const scroller = document.getElementById('strip')!;
        const last = document.getElementById(lastId)!;
        const available = scroller.clientWidth - 8;
        scroller.scrollLeft = last.offsetLeft + last.offsetWidth - available;
        const centre = (el: Element) => {
          const r = el.getBoundingClientRect();
          return document.elementFromPoint((r.left + r.right) / 2, (r.top + r.bottom) / 2);
        };
        const plusEl = document.getElementById('plus')!;
        const moreEl = document.getElementById('more')!;
        const box = (el: Element) => {
          const r = el.getBoundingClientRect();
          return { left: r.left, right: r.right };
        };
        return {
          overflowing: scroller.scrollWidth - scroller.clientWidth > 1,
          plusHit: centre(plusEl)?.id ?? null,
          moreHit: centre(moreEl)?.id ?? null,
          plus: box(plusEl),
          more: box(moreEl),
        };
      },
      `t${TAB_COUNT - 1}`
    );
  };

  it('control: at the end of the scroll content, the "+" is under the ▾', async () => {
    if (!browser) return;
    const result = await probe('inside');
    expect(result.overflowing).toBe(true);
    expect(result.plusHit).not.toBe('plus');
  });

  it('in its own slot, the "+" takes its press and sits 4px before the ▾', async () => {
    if (!browser) return;
    const result = await probe('outside');
    expect(result.overflowing).toBe(true);
    expect(result.plusHit).toBe('plus');
    expect(result.moreHit).toBe('more');
    expect(result.more.left - result.plus.right).toBe(4);
  });
});
