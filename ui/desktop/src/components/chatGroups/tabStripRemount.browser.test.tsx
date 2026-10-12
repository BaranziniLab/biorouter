// @vitest-environment node
/**
 * A tab switch leaves the chat tab strip where it was (vision V2-R7-1).
 *
 * The shell keys a tab's whole chat on the active tab id, and the strip renders in that chat's
 * header, so every tab switch mounts a NEW strip (see `TabStripMemory`). The group's memory kept
 * the tab offsets and order but not the scroll: each new strip started at scroll 0 and the reveal
 * then carried the active tab to the strip's end. Measured in the running app at 1100x760 with 14
 * tabs scrolled to their end: 'Initial greeting', clicked at 347..483, came to rest at 732..868
 * with the pointer over another tab, so a second click landed on a different chat.
 *
 * jsdom lays nothing out, so the real strip is bundled with esbuild, `styles/main.css` is compiled
 * by Tailwind with the classes the bundle uses (plus `chat-tabs.css`), and Chromium clicks it. The
 * harness remounts the strip on every switch with one shared memory, exactly as the shell does.
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
 * `window.__TABS` tabs, the last one active, in a band `--band-width` wide. The strip is keyed on
 * the active tab and given one memory for its whole life, as `ChatGroupsShell` does.
 */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ChatTabStrip } from './ChatTabStrip';
import { createTabStripMemory } from './tabStripMemory';

const w = window as unknown as Record<string, unknown>;
const count = (w.__TABS as number) ?? 14;
const tabs = Array.from({ length: count }, (_, i) => ({
  tabId: 'tab-' + i,
  sessionId: 's' + i,
  title: 'A fairly long chat title ' + i,
  userSetName: false,
}));
const memory = createTabStripMemory();

function Harness() {
  const [active, setActive] = React.useState(tabs[count - 1].tabId);
  return (
    <div id="band" style={{ display: 'flex', height: 44, width: 'var(--band-width)' }}>
      <ChatTabStrip
        key={active}
        memory={memory}
        tabs={tabs}
        activeTabId={active}
        runningSessionIds={[]}
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

describe('a tab switch remounts the strip and leaves it where it was (V2-R7-1)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the tab strip remount tests.');
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

  /** Two frames, every running animation finished, and a pause. */
  const settle = (page: Page, ms = 150) =>
    page.evaluate(
      (wait) =>
        new Promise<void>((done) =>
          requestAnimationFrame(() =>
            requestAnimationFrame(async () => {
              await Promise.all(
                document
                  .getAnimations()
                  .filter((a) => Number.isFinite(a.effect?.getComputedTiming().endTime ?? 0))
                  .map((a) => a.finished.catch(() => undefined))
              );
              setTimeout(done, wait);
            })
          )
        ),
      ms
    );

  /** Hold on to the strip element now on the page, to tell a remount from a re-render. */
  const holdStrip = (page: Page) =>
    page.evaluate((s) => {
      (window as unknown as { __held: Element }).__held = document.querySelector(s)!;
    }, STRIP);
  const heldIsConnected = (page: Page) =>
    page.evaluate(() => (window as unknown as { __held: Element }).__held.isConnected);

  /**
   * Where a tab is laid out on screen: the strip's box plus its offset less the scroll, which no
   * select or entrance animation (scale, translate) moves.
   */
  const laidOut = (page: Page, tabId: string) =>
    page.evaluate(
      ([s, id]) => {
        const strip = document.querySelector<HTMLElement>(s)!;
        const tab = strip.querySelector<HTMLElement>(`[data-tab-id="${id}"]`)!;
        const left = strip.getBoundingClientRect().left + tab.offsetLeft - strip.scrollLeft;
        return { left, right: left + tab.offsetWidth, scrollLeft: strip.scrollLeft };
      },
      [STRIP, tabId] as const
    );

  /** The tab under a point, as a pointer would hit it. */
  const tabAt = (page: Page, x: number, y: number) =>
    page.evaluate(
      ([px, py]) =>
        (document.elementFromPoint(px, py)?.closest('[data-tab-id]') as HTMLElement | null)?.dataset
          .tabId ?? null,
      [x, y] as const
    );

  /** Tabs wholly clear of both edge fades as the strip sits now, with the middle of each. */
  const clearTabs = (page: Page) =>
    page.evaluate((s) => {
      const el = document.querySelector<HTMLElement>(s)!;
      const strip = el.getBoundingClientRect();
      const fade = parseFloat(getComputedStyle(el).getPropertyValue('--tab-edge-fade')) || 0;
      const out: Array<{ id: string; x: number; y: number }> = [];
      for (const tab of el.querySelectorAll<HTMLElement>('[data-tab-id]')) {
        const box = tab.getBoundingClientRect();
        if (box.left >= strip.left + fade + 1 && box.right <= strip.right - fade - 1) {
          out.push({
            id: tab.dataset.tabId!,
            x: box.left + box.width / 2,
            y: box.top + box.height / 2,
          });
        }
      }
      return out;
    }, STRIP);

  it('a tab clicked in view stays under the pointer, and so does the next one', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(14, 830);
    try {
      const opened = await laidOut(page, 'tab-13');
      expect(
        opened.scrollLeft,
        '14 tabs overflow an 830px band; opened at its end'
      ).toBeGreaterThan(1000);
      // The first tab clear of the start fade, then the last one clear of the end fade.
      for (const pick of ['first', 'last'] as const) {
        const clear = (await clearTabs(page)).filter((t) => t.id !== 'tab-13');
        const target = pick === 'first' ? clear[0] : clear[clear.length - 1];
        expect(target, 'a tab clear of both fades').toBeDefined();
        const before = await laidOut(page, target.id);
        await holdStrip(page);
        await page.mouse.click(target.x, target.y);
        await settle(page);
        expect(await heldIsConnected(page), 'the switch mounted a new strip').toBe(false);
        const after = await laidOut(page, target.id);
        expect(after.scrollLeft, 'the new strip is scrolled where the old one was').toBe(
          before.scrollLeft
        );
        expect(Math.abs(after.left - before.left), `${target.id} did not move`).toBeLessThanOrEqual(
          0.5
        );
        expect(await tabAt(page, target.x, target.y), 'the pointer is still over it').toBe(
          target.id
        );
        expect(
          await page
            .locator(`[data-tab-id="${target.id}"] [role="tab"]`)
            .getAttribute('aria-selected')
        ).toBe('true');
      }
    } finally {
      await page.close();
    }
  });

  it('a tab the start cuts is still revealed clear of the start fade after the remount', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(14, 830);
    try {
      // Scroll back until the strip's start cuts 30px off tab-4, then click the part of its title
      // button still in view (the start fade masks it but does not stop the click).
      await page.evaluate((s) => {
        const strip = document.querySelector<HTMLElement>(s)!;
        const tab = strip.querySelector<HTMLElement>('[data-tab-id="tab-4"]')!;
        strip.scrollLeft = tab.offsetLeft + 30;
      }, STRIP);
      await settle(page);
      const cut = await page.evaluate((s) => {
        const strip = document.querySelector<HTMLElement>(s)!.getBoundingClientRect();
        for (const el of document.querySelectorAll<HTMLElement>(`${s} [data-tab-id]`)) {
          const tab = el.getBoundingClientRect();
          const button = el.querySelector('[role="tab"]')!.getBoundingClientRect();
          if (tab.left < strip.left - 4 && button.right > strip.left + 16) {
            return {
              id: el.dataset.tabId!,
              x: (strip.left + button.right) / 2,
              y: button.top + button.height / 2,
            };
          }
        }
        return null;
      }, STRIP);
      expect(cut, 'the strip start cuts through a tab').not.toBeNull();
      const before = await laidOut(page, cut!.id);
      await holdStrip(page);
      await page.mouse.click(cut!.x, cut!.y);
      await settle(page);
      expect(await heldIsConnected(page), 'the switch mounted a new strip').toBe(false);
      const after = await laidOut(page, cut!.id);
      const stripLeft = await page.evaluate(
        (s) => document.querySelector(s)!.getBoundingClientRect().left,
        STRIP
      );
      // From the remembered scroll, the reveal moves it only as far as the fade's inner edge.
      expect(after.left - stripLeft).toBeGreaterThanOrEqual(31.5);
      expect(after.left - stripLeft).toBeLessThanOrEqual(32.5);
      expect(after.scrollLeft).toBeLessThan(before.scrollLeft);
      expect(
        before.scrollLeft - after.scrollLeft,
        'a small move, not a trip to an end'
      ).toBeLessThan(120);
    } finally {
      await page.close();
    }
  });
});
