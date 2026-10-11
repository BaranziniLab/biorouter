// @vitest-environment node
/**
 * The sidebar's open and close motion and its overlay shadow, run in a real
 * engine (spec 3.4 "Sidebar collapse motion", principle 9, checklist 7 and 12).
 *
 * jsdom runs no CSS and starts no transition, so a rule can read right and do
 * nothing: the panel's transition named `transform` while Tailwind v4 moves it
 * with the separate `translate` property, so the panel jumped while the gap
 * beside it eased (vision V1-H3), and the overlay's shadow stayed on the closed
 * panel and bled down the window's left edge (V1-M1, V3-24). This loads the
 * WS-SIDEBAR blocks and WS-TOKENS' resize rule straight out of `main.css` into
 * Chromium and watches what actually animates.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const SIDEBAR_TSX = readFileSync(resolve(here, 'sidebar.tsx'), 'utf8');

function block(owner: string, topic: string): string {
  const begin = `/* @ws ${owner} begin: ${topic} */`;
  const end = `/* @ws ${owner} end: ${topic} */`;
  const from = MAIN_CSS.indexOf(begin);
  const to = MAIN_CSS.indexOf(end);
  if (from < 0 || to < from) throw new Error(`main.css has no "${owner}: ${topic}" block`);
  return MAIN_CSS.slice(from, to + end.length);
}

function token(name: string): string {
  const match = MAIN_CSS.match(new RegExp(`${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
  if (!match) throw new Error(`main.css does not declare ${name}`);
  return `${name}: ${match[1].trim()};`;
}

/**
 * What the Tailwind utilities on the three boxes compile to. ⚠ Tailwind 4 writes
 * `translate-x-[-100%]` as `--tw-translate-x: -100%; translate: var(--tw-translate-x)
 * var(--tw-translate-y)`, the individual property, not `transform`. That is the
 * fact the panel's transition has to name, and `sidebar.tsx` is checked below
 * for still moving the panel with that utility.
 */
const UTILITIES = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; }
:root { --sidebar-width: 288px; --tw-translate-x: 0; --tw-translate-y: 0; }
[data-slot='sidebar-gap'] { position: relative; width: var(--sidebar-width); height: 10px; }
[data-slot='sidebar'][data-collapsible='offcanvas'] > [data-slot='sidebar-gap'] { width: 0; }
[data-slot='sidebar-container'] { position: fixed; top: 0; bottom: 0; left: 0; width: var(--sidebar-width); z-index: 10; background: #f7f7f5; }
[data-slot='sidebar'][data-collapsible='offcanvas'] > [data-slot='sidebar-container'] {
  --tw-translate-x: -100%;
  translate: var(--tw-translate-x) var(--tw-translate-y);
}
`;

const COMPACT_GAP = `
body.biorouter-sidebar-compact [data-slot='sidebar-gap'] { width: 0 !important; }
`;

/** The global reduced-motion reset, word for word (it carries no block marker). */
const REDUCED_MOTION = `@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
    scroll-behavior: auto !important;
  }
}`;

const PAGE =
  '<!doctype html><html><head><style>' +
  [
    `:root { ${token('--dur-med')} ${token('--dur-fast-max')} ${token('--ease-out')} ${token(
      '--shadow-popover'
    )} ${token('--z-sticky')} }`,
    UTILITIES,
    COMPACT_GAP,
    REDUCED_MOTION,
    block('WS-SIDEBAR', 'sidebar open and close motion'),
    block('WS-SIDEBAR', 'overlay shadow only while open'),
    block('WS-TOKENS', 'resize rule'),
  ].join('\n') +
  '</style></head><body>' +
  '<div data-slot="sidebar" data-state="expanded" data-collapsible="" data-side="left">' +
  '<div data-slot="sidebar-gap" data-motion-layout=""></div>' +
  '<div data-slot="sidebar-container" data-motion-layout=""></div>' +
  '</div></body></html>';

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

type Running = { property: string; duration: number };

describe('the sidebar opens and closes in a real engine', () => {
  let browser: Browser | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome) — skipping the sidebar motion ' +
          'tests. Run `npx playwright install chromium` to enable them.'
      );
    }
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1100, height: 600 } });
    await tab.setContent(PAGE);
    return tab;
  }

  /** Flip the state the way the provider does, and report what started on each box. */
  async function setState(tab: Page, state: 'expanded' | 'collapsed') {
    return tab.evaluate((next) => {
      const root = document.querySelector<HTMLElement>('[data-slot="sidebar"]')!;
      root.dataset.state = next;
      root.dataset.collapsible = next === 'collapsed' ? 'offcanvas' : '';
      const running = (slot: string): Running[] =>
        document
          .querySelector(`[data-slot="${slot}"]`)!
          .getAnimations()
          .map((animation) => ({
            property:
              (animation as Animation & { transitionProperty?: string }).transitionProperty ?? '',
            duration: Number(animation.effect?.getTiming().duration ?? 0),
          }));
      return { panel: running('sidebar-container'), gap: running('sidebar-gap') };
    }, state);
  }

  const panelX = (tab: Page) =>
    tab.evaluate(
      () => document.querySelector('[data-slot="sidebar-container"]')!.getBoundingClientRect().x
    );

  it('still moves the panel with the utility this test models', () => {
    expect(SIDEBAR_TSX).toContain('group-data-[collapsible=offcanvas]:translate-x-[-100%]');
  });

  it('models the reduced-motion reset main.css really has', () => {
    expect(MAIN_CSS).toContain(REDUCED_MOTION);
  });

  it('slides the panel with the gap: out over 175ms, in over 300ms', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      const closing = await setState(tab, 'collapsed');
      expect(closing.panel).toEqual([{ property: 'translate', duration: 175 }]);
      expect(closing.gap).toEqual([{ property: 'width', duration: 175 }]);
      // Mid-flight the panel is on its way, not already gone (the V1-H3 jump).
      await tab.waitForTimeout(40);
      const midway = await panelX(tab);
      expect(midway).toBeLessThan(0);
      expect(midway).toBeGreaterThan(-288);
      await tab.waitForTimeout(300);
      expect(await panelX(tab)).toBe(-288);

      const opening = await setState(tab, 'expanded');
      expect(opening.panel).toEqual([{ property: 'translate', duration: 300 }]);
      expect(opening.gap).toEqual([{ property: 'width', duration: 300 }]);
    } finally {
      await tab.close();
    }
  });

  it('snaps, with nothing animating, while the window is being resized', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      await tab.evaluate(() => document.body.classList.add('biorouter-window-resizing'));
      const closing = await setState(tab, 'collapsed');
      expect(closing.panel).toEqual([]);
      expect(closing.gap).toEqual([]);
      expect(await panelX(tab)).toBe(-288);
    } finally {
      await tab.close();
    }
  });

  it('wears the overlay shadow only while the overlay is open', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      const shadow = () =>
        tab.evaluate(
          () =>
            getComputedStyle(document.querySelector('[data-slot="sidebar-container"]')!).boxShadow
        );
      await tab.evaluate(() => document.body.classList.add('biorouter-sidebar-compact'));
      expect(await shadow()).not.toBe('none');

      await setState(tab, 'collapsed');
      await tab.waitForTimeout(250);
      // Translated to x=-288 and painted above the page: any shadow here is a grey
      // band down the window's left edge.
      expect(await shadow()).toBe('none');
      const leftEdge = await tab.evaluate(() => {
        const panel = document.querySelector('[data-slot="sidebar-container"]')!;
        return panel.getBoundingClientRect().right;
      });
      expect(leftEdge).toBe(0);
    } finally {
      await tab.close();
    }
  });

  /**
   * Vision V1-R2-9. Keyed on the state alone, the shadow went in the first collapsed frame while
   * the panel was still at x=0, so the overlay's edge went flat in one frame and the panel slid out
   * flat. Closing, the shadow now stays until the slide ends; opening, it is there at once.
   */
  describe('the overlay keeps its shadow while it slides out', () => {
    const shadow = (tab: Page) =>
      tab.evaluate(
        () => getComputedStyle(document.querySelector('[data-slot="sidebar-container"]')!).boxShadow
      );
    const compact = (tab: Page) =>
      tab.evaluate(() => document.body.classList.add('biorouter-sidebar-compact'));

    it('drops the shadow only once the panel has left, and wears it from the first frame open', async () => {
      if (!browser) return;
      const tab = await open();
      try {
        await compact(tab);
        // Open and drawn, lifted off the page (this also settles the style a close starts from).
        expect(await shadow(tab)).not.toBe('none');
        await setState(tab, 'collapsed');
        // The first collapsed frame: the panel has barely moved, and its edge is still lifted.
        expect(await shadow(tab)).not.toBe('none');
        await tab.waitForTimeout(80);
        const midway = await panelX(tab);
        expect(midway).toBeLessThan(0);
        expect(midway).toBeGreaterThan(-288);
        expect(await shadow(tab)).not.toBe('none');
        await tab.waitForTimeout(250);
        expect(await panelX(tab)).toBe(-288);
        expect(await shadow(tab)).toBe('none');

        await setState(tab, 'expanded');
        expect(await shadow(tab)).not.toBe('none');
      } finally {
        await tab.close();
      }
    });

    it('drops it with the panel when nothing slides: a resize, or reduced motion', async () => {
      if (!browser) return;
      for (const setup of ['resizing', 'reduced'] as const) {
        const tab = await open();
        try {
          await compact(tab);
          expect(await shadow(tab), setup).not.toBe('none');
          if (setup === 'resizing') {
            await tab.evaluate(() => document.body.classList.add('biorouter-window-resizing'));
          } else {
            await tab.emulateMedia({ reducedMotion: 'reduce' });
          }
          await setState(tab, 'collapsed');
          await tab.waitForTimeout(40);
          // Well inside the 175ms a slide would take: the panel is gone, and so is its shadow.
          expect(await panelX(tab), setup).toBe(-288);
          expect(await shadow(tab), setup).toBe('none');
        } finally {
          await tab.close();
        }
      }
    });
  });
});
