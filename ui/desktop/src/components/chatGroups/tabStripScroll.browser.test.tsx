// @vitest-environment node
/**
 * The chat tab strip scrolls, and a scroll stays where it was put (vision V1-R5-1, the strip's
 * D-32 ladder: shrink, then scroll, then the ▾).
 *
 * The strip is `overflow-x: auto`, but every scroll was pulled back to the offset that shows the
 * active tab within a frame or two. The strip's own scroll re-renders it (the edge fade's flags
 * flip in `useTabStripScrolledEdges`), and `ChatTabStrip` revealed the active tab on EVERY render,
 * so the reveal undid the scroll that caused the render: 10 tabs read back scrollLeft 451 50 ms
 * after being set to 0, and a strip 34px too wide shook between its two ends for as long as the
 * wheel ran. jsdom lays nothing out and fires no scroll for a write to `scrollLeft`, so the
 * real strip is bundled with esbuild, `styles/main.css` is compiled by Tailwind with the classes
 * the bundle uses (plus `chat-tabs.css`, which the strip imports), and Chromium scrolls it.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

/**
 * The strip as ChatGroupsShell mounts it, bare (it is pure props): `window.__TABS` tabs, the last
 * one active, in a band `window.__WIDTH` wide. `__select` makes another tab active and
 * `__rerender` re-renders the parent without moving any tab (a running dot comes and goes), the
 * way a streaming reply re-renders the shell.
 */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ChatTabStrip } from './ChatTabStrip';

const w = window as unknown as Record<string, unknown>;
const count = (w.__TABS as number) ?? 10;
const tabs = Array.from({ length: count }, (_, i) => ({
  tabId: 'tab-' + i,
  sessionId: 's' + i,
  title: 'A fairly long chat title ' + i,
  userSetName: false,
}));

function Harness() {
  const [active, setActive] = React.useState(tabs[count - 1].tabId);
  const [tick, setTick] = React.useState(0);
  w.__select = setActive;
  w.__rerender = () => setTick((t) => t + 1);
  return (
    <div id="band" style={{ display: 'flex', height: 44, width: 'var(--band-width)' }}>
      <ChatTabStrip
        tabs={tabs}
        activeTabId={active}
        runningSessionIds={tick % 2 === 1 ? ['s0'] : []}
        onSelect={setActive}
        onClose={() => {}}
        onReorder={() => {}}
        reserveTitlebar={false}
        isCompactSidebarOverlayOpen={false}
      />
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<Harness />);
`;

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

const STRIP = '[role="tablist"]';

describe('a chat tab strip that overflows scrolls, and the scroll holds (V1-R5-1)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the tab strip scroll tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"' },
      loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    css =
      compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])) +
      '\n' +
      readFileSync(resolve(here, 'chat-tabs.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(tabs: number, width: number): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width: 1440, height: 300 } });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.setContent(
      `<!doctype html><html><head><style>${css}</style>` +
        `<style>:root { --band-width: ${width}px; }</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: `window.__TABS = ${tabs};` });
    await page.addScriptTag({ content: script });
    await page.waitForSelector(`${STRIP} [data-tab-id]`);
    await settle(page);
    return page;
  }

  /** Two frames and a pause: whatever a render or a scroll schedules has run. */
  const settle = (page: Page, ms = 150) =>
    page.evaluate(
      (wait) =>
        new Promise<void>((done) =>
          requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(done, wait)))
        ),
      ms
    );

  const geometry = (page: Page) =>
    page.evaluate((s) => {
      const strip = document.querySelector<HTMLElement>(s)!;
      return {
        scrollLeft: strip.scrollLeft,
        overflow: strip.scrollWidth - strip.clientWidth,
        start: strip.hasAttribute('data-scrolled-start'),
        end: strip.hasAttribute('data-scrolled-end'),
      };
    }, STRIP);

  /** The strip's scrollLeft at every painted frame for `ms`, and how many scroll events fired. */
  const sample = (page: Page, ms: number) =>
    page.evaluate(
      ([s, duration]) =>
        new Promise<{ frames: number[]; scrolls: number }>((done) => {
          const strip = document.querySelector<HTMLElement>(s as string)!;
          const frames: number[] = [];
          let scrolls = 0;
          const onScroll = () => (scrolls += 1);
          strip.addEventListener('scroll', onScroll);
          const end = performance.now() + (duration as number);
          const tick = () => {
            frames.push(Math.round(strip.scrollLeft));
            if (performance.now() < end) requestAnimationFrame(tick);
            else {
              strip.removeEventListener('scroll', onScroll);
              done({ frames, scrolls });
            }
          };
          requestAnimationFrame(tick);
        }),
      [STRIP, ms] as const
    );

  /** Is the tab fully inside the strip's visible box? */
  const inView = (page: Page, tabId: string) =>
    page.evaluate(
      ([s, id]) => {
        const strip = document.querySelector<HTMLElement>(s)!.getBoundingClientRect();
        const tab = document.querySelector(`[data-tab-id="${id}"]`)!.getBoundingClientRect();
        return tab.left >= strip.left - 0.5 && tab.right <= strip.right + 0.5;
      },
      [STRIP, tabId] as const
    );

  /** Where a tab sits against the strip's box, and the fade the strip declares and wears. */
  const placement = (page: Page, tabId: string) =>
    page.evaluate(
      ([s, id]) => {
        const el = document.querySelector<HTMLElement>(s)!;
        const strip = el.getBoundingClientRect();
        const tab = document.querySelector(`[data-tab-id="${id}"]`)!.getBoundingClientRect();
        return {
          fade: parseFloat(getComputedStyle(el).getPropertyValue('--tab-edge-fade')),
          start: el.hasAttribute('data-scrolled-start'),
          end: el.hasAttribute('data-scrolled-end'),
          fromStart: tab.left - strip.left,
          toEnd: strip.right - tab.right,
        };
      },
      [STRIP, tabId] as const
    );

  /** The tab an edge of the strip cuts through as it sits now, and the middle of its visible part. */
  const cutBy = (page: Page, edge: 'start' | 'end') =>
    page.evaluate(
      ([s, side]) => {
        const strip = document.querySelector<HTMLElement>(s)!.getBoundingClientRect();
        const line = side === 'end' ? strip.right : strip.left;
        for (const el of document.querySelectorAll<HTMLElement>(`${s} [data-tab-id]`)) {
          const tab = el.getBoundingClientRect();
          if (tab.left < line - 4 && tab.right > line + 4) {
            const from = Math.max(tab.left, strip.left);
            const to = Math.min(tab.right, strip.right);
            return { id: el.dataset.tabId!, x: (from + to) / 2, y: tab.top + tab.height / 2 };
          }
        }
        return null;
      },
      [STRIP, edge] as const
    );

  // Vision V2-R4-4: selecting a tab the strip's end cut through revealed it to 8px from the end,
  // inside the 32px end fade, so its last 24px, the close slot and the end of its accent strip
  // were faded out (1100x760, 15 tabs).
  it('a tab selected where an end cuts it is revealed clear of that end fade', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(15, 830);
    try {
      await page.evaluate((s) => (document.querySelector<HTMLElement>(s)!.scrollLeft = 0), STRIP);
      await settle(page);
      const cut = await cutBy(page, 'end');
      expect(cut, 'the strip end cuts through a tab').not.toBeNull();
      await page.mouse.click(cut!.x, cut!.y);
      await settle(page);
      const shown = await placement(page, cut!.id);
      expect(shown.fade, 'the strip fades its ends over 32px').toBe(32);
      expect(shown.end, 'tabs remain past the end, so the end still fades').toBe(true);
      expect(shown.toEnd, 'the tab ends at the fade, not inside it').toBeGreaterThanOrEqual(31.5);
      expect(shown.toEnd, 'and no further than it needs to').toBeLessThanOrEqual(32.5);
      expect(shown.fromStart, 'its start is clear of the start fade').toBeGreaterThanOrEqual(31.5);
    } finally {
      await page.close();
    }
  });

  it('a tab selected where the start cuts it is revealed clear of the start fade', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(15, 830);
    try {
      // Opened at the end (the last tab is active); scroll back until the start cuts a tab.
      await page.evaluate(
        (s) => (document.querySelector<HTMLElement>(s)!.scrollLeft -= 300),
        STRIP
      );
      await settle(page);
      const cut = await cutBy(page, 'start');
      expect(cut, 'the strip start cuts through a tab').not.toBeNull();
      await page.evaluate(
        (id) => (window as unknown as { __select: (id: string) => void }).__select(id),
        cut!.id
      );
      await settle(page);
      const shown = await placement(page, cut!.id);
      expect(shown.start).toBe(true);
      expect(shown.fromStart, 'the tab starts at the fade, not inside it').toBeGreaterThanOrEqual(
        31.5
      );
      expect(shown.fromStart).toBeLessThanOrEqual(32.5);
      expect(shown.toEnd, 'its end is clear of the end fade').toBeGreaterThanOrEqual(31.5);
    } finally {
      await page.close();
    }
  });

  it('the last tab is revealed at the end, where the strip stops fading', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(15, 830);
    try {
      await page.evaluate((s) => (document.querySelector<HTMLElement>(s)!.scrollLeft = 0), STRIP);
      await settle(page);
      await page.evaluate(() =>
        (window as unknown as { __select: (id: string) => void }).__select('tab-13')
      );
      await settle(page);
      await page.evaluate(() =>
        (window as unknown as { __select: (id: string) => void }).__select('tab-14')
      );
      await settle(page);
      const { scrollLeft, overflow } = await geometry(page);
      expect(scrollLeft).toBe(overflow);
      const shown = await placement(page, 'tab-14');
      expect(shown.end, 'nothing past the end, so it does not fade').toBe(false);
      expect(shown.toEnd).toBeGreaterThanOrEqual(-0.5);
    } finally {
      await page.close();
    }
  });

  it('reveals the active tab on open, then holds a scroll set on the strip', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(10, 1100);
    try {
      const opened = await geometry(page);
      expect(opened.overflow, '10 tabs overflow a 1100px band').toBeGreaterThan(200);
      expect(opened.scrollLeft, 'the last (active) tab is revealed').toBe(opened.overflow);
      expect(await inView(page, 'tab-9')).toBe(true);

      await page.evaluate((s) => (document.querySelector<HTMLElement>(s)!.scrollLeft = 0), STRIP);
      const { frames } = await sample(page, 400);
      expect(new Set(frames), 'every painted frame is at the scroll that was set').toEqual(
        new Set([0])
      );
      const after = await geometry(page);
      // The edge flags flipped, so the strip DID re-render on its own scroll; the scroll held.
      expect(after).toMatchObject({ scrollLeft: 0, start: false, end: true });
      expect(await inView(page, 'tab-0')).toBe(true);
    } finally {
      await page.close();
    }
  });

  it('scrolls with the wheel and stays put', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(10, 1100);
    try {
      const box = (await page.locator(STRIP).boundingBox())!;
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      const before = (await geometry(page)).scrollLeft;
      for (let i = 0; i < 6; i += 1) {
        await page.mouse.wheel(-60, 0);
        await page.waitForTimeout(60);
      }
      await settle(page, 300);
      const scrolled = (await geometry(page)).scrollLeft;
      expect(scrolled, 'six ticks of -60px moved the strip left').toBeLessThan(before - 200);
      const { frames } = await sample(page, 400);
      expect(new Set(frames), 'and it stays where the wheel left it').toEqual(new Set([scrolled]));
    } finally {
      await page.close();
    }
  });

  it('does not shake when the overflow is small', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(7, 700);
    try {
      // Widen the band until the strip overflows by 34px, as measured in the running app. Every
      // tab is at its floor while the strip overflows, so the overflow falls 1:1 with the width.
      const { overflow } = await geometry(page);
      expect(overflow, '7 tabs overflow a 700px band').toBeGreaterThan(34);
      await page.addStyleTag({
        content: `:root { --band-width: ${700 + overflow - 34}px !important; }`,
      });
      await settle(page);
      const small = await geometry(page);
      expect(Math.abs(small.overflow - 34)).toBeLessThanOrEqual(1);
      expect(small.scrollLeft, 'the active tab is revealed after the resize').toBe(small.overflow);

      const box = (await page.locator(STRIP).boundingBox())!;
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      for (let i = 0; i < 3; i += 1) {
        await page.mouse.wheel(-40, 0);
        await page.waitForTimeout(120);
      }
      await settle(page, 300);
      const { frames, scrolls } = await sample(page, 800);
      expect(new Set(frames), 'one offset, held: no frame at the other end').toEqual(new Set([0]));
      expect(scrolls, 'no scroll events once the wheel stopped').toBe(0);
    } finally {
      await page.close();
    }
  });

  it('a parent re-render keeps the scroll; selecting a hidden tab still reveals it', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(10, 1100);
    try {
      await page.evaluate((s) => (document.querySelector<HTMLElement>(s)!.scrollLeft = 0), STRIP);
      await settle(page);
      for (let i = 0; i < 3; i += 1) {
        await page.evaluate(() => (window as unknown as { __rerender: () => void }).__rerender());
        await settle(page, 50);
      }
      expect((await geometry(page)).scrollLeft, 'a re-render that moves no tab').toBe(0);

      await page.evaluate(() =>
        (window as unknown as { __select: (id: string) => void }).__select('tab-8')
      );
      await settle(page);
      expect(await inView(page, 'tab-8'), 'another tab made active is revealed').toBe(true);
      expect((await geometry(page)).scrollLeft).toBeGreaterThan(0);
    } finally {
      await page.close();
    }
  });
});
