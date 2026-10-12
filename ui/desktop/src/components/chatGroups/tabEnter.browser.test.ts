// @vitest-environment node
/**
 * The new-tab entrance, measured in a real engine (spec 3.16, 5.7).
 *
 * jsdom runs no CSS animation, so `ChatTabStrip.flip.test.tsx` can only pin the
 * attribute's life and the rule's text. Here the real rules from `main.css`
 * (the active tab's settle, the entrance, and both keyframes) run in Chromium:
 *
 *   - a new tab grows in from its LEFT edge (its left x does not move while it
 *     scales), starting transparent;
 *   - on an active new tab the entrance wins over the select settle;
 *   - and why an active tab takes `data-tab-entered` when its entrance ends:
 *     dropping `data-tab-entering` alone would start `br-tab-select`, a second
 *     settle, and keeping the entrance would replay it whenever React moves the
 *     tab with `insertBefore` (a reorder).
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

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

const TAB_CSS = [
  ".br-tab[data-active='true'] {",
  '@keyframes br-tab-select {',
  ".br-tab[data-tab-entering='true'] {",
  ".br-tab[data-active='true'][data-tab-entered='true'] {",
  '@keyframes br-tab-enter {',
]
  .map(block)
  .join('\n');

const TOKENS = `:root {
  --dur-fast-max: 175ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
  --ease-spring: cubic-bezier(0.34, 1.56, 0.64, 1);
  --background-default: #fff;
  --accent-bar: #cf6d47;
  --text-default: #222;
}
body { margin: 0; font: 13px Arial, sans-serif; }
.strip { display: flex; gap: 8px; padding: 0 8px; height: 44px; align-items: center; }
.br-tab { display: flex; align-items: center; height: 32px; width: 160px; padding: 0 11px; }`;

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

describe('a new tab grows in from its left edge', () => {
  let browser: Browser | null = null;
  let page: Page;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found — skipping the tab entrance tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
      return;
    }
    page = await browser.newPage({ viewport: { width: 800, height: 200 } });
    await page.setContent(
      `<!doctype html><html><head><style>${TOKENS}</style><style>${TAB_CSS}</style></head>` +
        `<body><div class="strip" id="strip"><div class="br-tab" id="a">One</div></div></body></html>`
    );
  }, 180_000);

  // Closing Chromium under a loaded full run overran the 30 s default hook budget.
  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Add tab B as `ChatTabStrip` does for an opened tab, and freeze its animations at `t` ms. */
  const openTab = (active: boolean, t: number) =>
    page.evaluate(
      ({ active: isActive, t: at }) => {
        document.getElementById('b')?.remove();
        const tab = document.createElement('div');
        tab.className = 'br-tab';
        tab.id = 'b';
        tab.textContent = 'Two';
        if (isActive) tab.dataset.active = 'true';
        tab.dataset.tabEntering = 'true';
        document.getElementById('strip')!.appendChild(tab);
        const animations = tab.getAnimations();
        for (const animation of animations) {
          animation.pause();
          animation.currentTime = at;
        }
        const style = getComputedStyle(tab);
        return {
          names: animations.map((a) => (a as Animation & { animationName: string }).animationName),
          left: tab.getBoundingClientRect().left,
          width: tab.getBoundingClientRect().width,
          opacity: Number(style.opacity),
          origin: style.transformOrigin,
        };
      },
      { active, t }
    );

  it('starts transparent at 96% scale and its left edge never moves', async () => {
    if (!browser) return;
    const start = await openTab(true, 0);
    const end = await openTab(true, 175);
    expect(start.opacity).toBe(0);
    expect(start.width).toBeCloseTo(end.width * 0.96, 1);
    // Grown from the left edge: the left x is the same at the start and the end.
    expect(start.left).toBeCloseTo(end.left, 1);
    expect(start.origin.startsWith('0px')).toBe(true);
  });

  it('on an active new tab the entrance wins over the select settle', async () => {
    if (!browser) return;
    const active = await openTab(true, 0);
    expect(active.names).toEqual(['br-tab-enter']);
  });

  it('dropping the attribute from the active tab alone would start a second settle', async () => {
    if (!browser) return;
    const names = await page.evaluate(() => {
      const tab = document.getElementById('b')!;
      for (const animation of tab.getAnimations()) animation.finish();
      delete tab.dataset.tabEntering;
      return tab
        .getAnimations()
        .map((a) => (a as Animation & { animationName: string }).animationName);
    });
    // This is why ChatTabStrip swaps `entering` for `entered` on the active tab.
    expect(names).toEqual(['br-tab-select']);
  });

  it('swapping entering for entered on the active tab starts nothing, and a reorder replays nothing', async () => {
    if (!browser) return;
    const result = await page.evaluate(async () => {
      const names = (el: HTMLElement) =>
        el.getAnimations().map((a) => (a as Animation & { animationName: string }).animationName);
      // A fresh active tab, entrance finished, as ChatTabStrip's animationend leaves it.
      document.getElementById('b')?.remove();
      const tab = document.createElement('div');
      tab.className = 'br-tab';
      tab.id = 'b';
      tab.dataset.active = 'true';
      tab.dataset.tabEntering = 'true';
      document.getElementById('strip')!.appendChild(tab);
      for (const animation of tab.getAnimations()) animation.finish();
      delete tab.dataset.tabEntering;
      tab.dataset.tabEntered = 'true';
      const afterSwap = names(tab);

      // React reorders keyed tabs with insertBefore; a re-inserted element restarts
      // any CSS animation it carries.
      const strip = document.getElementById('strip')!;
      strip.insertBefore(tab, document.getElementById('a'));
      await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
      return { afterSwap, afterMove: names(tab), opacity: getComputedStyle(tab).opacity };
    });
    expect(result.afterSwap).toEqual([]);
    expect(result.afterMove).toEqual([]);
    expect(result.opacity).toBe('1');
  });
});
