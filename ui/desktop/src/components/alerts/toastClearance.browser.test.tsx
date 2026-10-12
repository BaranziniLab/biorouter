// @vitest-environment node
/**
 * A toast steps out of the way of find in chat and the Chat summary (vision V2-R7-4).
 *
 * In the running app "Chat ID copied" sat at 1004,44 to 1284,109 at 1300 wide, over the open
 * find bar (844,52 to 1284,94): its count, info tip, Aa, both arrows and Close were hidden. At
 * 1440 the same toast covered the docked summary rail's "To do" header and progress bar.
 * `toastClearance.ts` moves the stack below find, left of the summary popover, and into the
 * docked rail's column under its card. Stepped left of the rail it had covered the person's own
 * message instead (final app check, "Toasts do not cover transcript content").
 *
 * jsdom has no layout, so the real find bar (`SearchBar`, with `styles/search.css`), the real
 * react-toastify container as App.tsx configures it, and the watcher App.tsx starts are bundled
 * with esbuild, `styles/main.css` is compiled by Tailwind (the vendored ReactToastify.css after
 * it, as App imports them), and Chromium measures. The rail card and the popover are boxes with
 * their real class names and the geometry the app gives them (a 280px rail under the 44px
 * header with a 12px gutter, its card as tall as its content or, `tall`, as the rail; a 280px
 * popover 4px under the band, 16px in from the right).
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

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ToastContainer, toast } from 'react-toastify';
import { SearchBar } from '../conversation/SearchBar';
import { TooltipProvider } from '../ui/Tooltip';
import { NotificationContent, TOAST_SURFACE_CLASS_NAME } from './NotificationSurface';
import { watchToastClearance } from './toastClearance';

type Layout = { find: boolean; rail: boolean; tall: boolean; popover: boolean; split: boolean };
const NONE: Layout = { find: false, rail: false, tall: false, popover: false, split: false };

function Find() {
  return (
    <SearchBar
      onSearch={() => {}}
      onClose={() => {}}
      onNavigate={() => {}}
      initialSearchTerm="kdps"
      searchResults={{ count: 65, currentIndex: 1 }}
    />
  );
}

function Harness() {
  const [layout, setLayout] = React.useState<Layout>(NONE);
  (window as unknown as Record<string, unknown>).__layout = (patch: Partial<Layout>) =>
    setLayout((l) => ({ ...l, ...patch }));
  React.useEffect(() => watchToastClearance(), []);
  return (
    <TooltipProvider>
      <div style={{ position: 'fixed', inset: 0, display: 'flex', flexDirection: 'column' }}>
        <div style={{ height: 44, flex: 'none' }} />
        <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
          {layout.split && (
            <div id="left-cell" style={{ flex: 1, position: 'relative', minWidth: 0 }}>
              {layout.find && <Find />}
            </div>
          )}
          <div id="cell" style={{ flex: 1, position: 'relative', minWidth: 0 }}>
            {layout.find && !layout.split && <Find />}
          </div>
          {layout.rail && (
            <aside
              style={{
                width: 280,
                padding: '12px 12px 12px 0',
                display: 'flex',
                flexDirection: 'column',
                minHeight: 0,
                boxSizing: 'border-box',
              }}
            >
              <div
                className="br-summary-card"
                data-state="open"
                data-entered=""
                style={{ flex: layout.tall ? 1 : 'none', height: layout.tall ? undefined : 194, padding: 16 }}
              >
                To do 5 of 15
              </div>
            </aside>
          )}
        </div>
      </div>
      {layout.popover && (
        <div
          className="br-summary-popover biorouter-popover-surface"
          data-state="open"
          style={{ position: 'fixed', top: 48, right: 16, width: 280, height: 600 }}
        />
      )}
      <ToastContainer
        aria-label="Toast notifications"
        toastClassName={() => TOAST_SURFACE_CLASS_NAME}
        style={{
          '--toastify-z-index': 'var(--z-toast)',
          top: 'var(--toast-inset-top)',
          width: 'fit-content',
          minWidth: 'var(--toast-clear-width, 280px)',
          maxWidth: 'var(--toast-clear-width, min(420px, calc(100vw - 32px)))',
        } as React.CSSProperties}
        position="top-right"
        newestOnTop={false}
        closeOnClick={false}
        hideProgressBar
        icon={false}
      />
    </TooltipProvider>
  );
}

createRoot(document.getElementById('root')!).render(<Harness />);
(window as unknown as Record<string, unknown>).__toast = () =>
  toast(<NotificationContent status="success" title="Chat ID copied" message="20260904_1" />, {
    autoClose: false,
    toastId: 'chat-id',
  });
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

type Box = { left: number; top: number; right: number; bottom: number };
type Sample = { top: number; right: number; gliding: boolean };

const TOAST = '.Toastify__toast-container > *';
const FIND = '[data-testid="conversation-search-bar"]';
const CARD = '.br-summary-card';

const overlaps = (a: Box, b: Box) =>
  a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

describe('a toast steps out of the way of find in chat and the Chat summary (V2-R7-4)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the toast placement tests.');
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
      readFileSync(requireFromDesktop.resolve('react-toastify/dist/ReactToastify.css'), 'utf8') +
      '\n' +
      readFileSync(resolve(STYLES, 'search.css'), 'utf8') +
      '\n' +
      readFileSync(resolve(here, '../summary/summaryRail.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(
    width: number,
    { motion = 'no-preference' as 'no-preference' | 'reduce', theme = 'light' } = {}
  ): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width, height: 900 } });
    await page.emulateMedia({ reducedMotion: motion });
    await page.setContent(
      `<!doctype html><html class="${theme}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: script });
    await page.waitForFunction(() => '__layout' in window);
    return page;
  }

  /** Two frames, every finite animation finished, and a pause. */
  const settle = (page: Page, ms = 120) =>
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

  const setLayout = (page: Page, patch: Record<string, boolean>) =>
    page.evaluate(
      (p) => (window as unknown as { __layout: (p: object) => void }).__layout(p),
      patch
    );

  const showToast = async (page: Page) => {
    await page.evaluate(() => (window as unknown as { __toast: () => void }).__toast());
    await page.waitForSelector(TOAST);
  };

  const box = (page: Page, selector: string): Promise<Box> =>
    page.evaluate((s) => {
      const r = document.querySelector(s)!.getBoundingClientRect();
      return { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
    }, selector);

  /**
   * Run `action` in the page, then read the toast every painted frame for `ms`: its box, and
   * whether a `translate` transition is running on the container. Each read is a task queued from
   * the frame's callback, so it sees the frame after every animation-frame callback has run.
   */
  const watch = (page: Page, action: 'toast' | 'find-open' | 'find-close', ms = 450) =>
    page.evaluate(
      ({ act, duration }) =>
        new Promise<Sample[]>((done) => {
          const w = window as unknown as Record<string, (arg?: object) => void>;
          if (act === 'toast') w.__toast();
          else w.__layout({ find: act === 'find-open' });
          const out: Sample[] = [];
          const t0 = performance.now();
          const tick = () =>
            setTimeout(() => {
              const container = document.querySelector<HTMLElement>('.Toastify__toast-container');
              const card = container?.firstElementChild;
              if (container && card) {
                const r = card.getBoundingClientRect();
                out.push({
                  top: r.top,
                  right: r.right,
                  gliding: container
                    .getAnimations()
                    .some(
                      (a) =>
                        (a as Animation & { transitionProperty?: string }).transitionProperty ===
                          'translate' && a.playState === 'running'
                    ),
                });
              }
              if (performance.now() - t0 < duration) requestAnimationFrame(tick);
              else done(out);
            }, 0);
          requestAnimationFrame(tick);
        }),
      { act: action, duration: ms }
    );

  /** Each control in the find bar: does its centre take the pointer? */
  const findControlsReachable = (page: Page) =>
    page.evaluate((bar) => {
      const root = document.querySelector(bar)!;
      return [...root.querySelectorAll('input, button, [aria-live]')].map((el) => {
        const r = el.getBoundingClientRect();
        const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
        return {
          what: el.getAttribute('aria-label') ?? el.textContent ?? el.tagName,
          itself: !!hit && (hit === el || el.contains(hit) || root.contains(hit)),
          hit: hit?.outerHTML.slice(0, 80),
        };
      });
    }, FIND);

  it('a toast that arrives while find is open lands 8px below the bar, without sliding there', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1300);
    try {
      await setLayout(page, { find: true });
      await page.waitForSelector(FIND);
      await settle(page);
      const bar = await box(page, FIND);
      expect(bar.right).toBeCloseTo(1284, 0);

      const frames = await watch(page, 'toast');
      expect(frames.length).toBeGreaterThan(5);
      expect(
        frames.filter((f) => f.gliding),
        'the toast is placed, not slid'
      ).toEqual([]);
      // Its own entrance drops 8px into place, which starts exactly at the bar's bottom edge.
      for (const f of frames) expect(f.top).toBeGreaterThanOrEqual(bar.bottom - 0.5);

      await settle(page);
      const toast = await box(page, TOAST);
      expect(toast.top).toBeCloseTo(bar.bottom + 8, 0);
      expect(toast.right).toBeCloseTo(1284, 0);
      expect(overlaps(toast, bar)).toBe(false);
      for (const control of await findControlsReachable(page)) {
        expect(control.itself, `${control.what} hit ${control.hit}`).toBe(true);
      }
    } finally {
      await page.close();
    }
  });

  it('find opened under a standing toast: the toast glides below it, and back when find closes', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1300);
    try {
      await showToast(page);
      await settle(page);
      const before = await box(page, TOAST);
      expect(before.top).toBeCloseTo(44, 0);
      expect(before.right).toBeCloseTo(1284, 0);

      const down = await watch(page, 'find-open');
      await settle(page);
      const bar = await box(page, FIND);
      const after = await box(page, TOAST);
      expect(after.top).toBeCloseTo(bar.bottom + 8, 0);
      expect(after.right).toBeCloseTo(1284, 0);
      expect(overlaps(after, bar)).toBe(false);
      for (const control of await findControlsReachable(page)) {
        expect(control.itself, `${control.what} hit ${control.hit}`).toBe(true);
      }
      // A glide, not a jump: a running transition, frames between the two ends, never past them.
      expect(down.some((f) => f.gliding)).toBe(true);
      const between = down.filter((f) => f.top > before.top + 1 && f.top < after.top - 1);
      expect(between.length, down.map((f) => f.top.toFixed(1)).join(' ')).toBeGreaterThan(2);
      for (let i = 1; i < down.length; i++) {
        expect(down[i].top).toBeGreaterThanOrEqual(down[i - 1].top - 0.5);
        expect(down[i].top).toBeLessThanOrEqual(after.top + 0.5);
      }

      await setLayout(page, { find: false });
      await settle(page, 300);
      const back = await box(page, TOAST);
      expect(back.top).toBeCloseTo(44, 0);
      expect(back.right).toBeCloseTo(1284, 0);
    } finally {
      await page.close();
    }
  });

  it('with the summary rail docked the toast joins its column, under the card, never over the conversation (1440)', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1440);
    try {
      await setLayout(page, { rail: true });
      await showToast(page);
      await settle(page);
      const card = await box(page, CARD);
      const conversation = await box(page, '#cell');
      expect(card.top).toBeCloseTo(56, 0);
      expect(card.bottom).toBeCloseTo(250, 0);
      const toast = await box(page, TOAST);
      expect(toast.top).toBeCloseTo(card.bottom + 8, 0);
      expect(toast.left).toBeCloseTo(card.left, 0);
      expect(toast.right).toBeCloseTo(card.right, 0);
      expect(overlaps(toast, card)).toBe(false);
      expect(overlaps(toast, conversation), 'the toast stays off the conversation').toBe(false);

      // Find opened in the conversation beside the rail: the toast is already clear of it.
      await setLayout(page, { find: true });
      await page.waitForSelector(FIND);
      await settle(page, 300);
      const bar = await box(page, FIND);
      const both = await box(page, TOAST);
      expect(both).toEqual(toast);
      expect(overlaps(both, bar)).toBe(false);
      for (const control of await findControlsReachable(page)) {
        expect(control.itself, `${control.what} hit ${control.hit}`).toBe(true);
      }

      // The rail folds away: the toast goes back to its corner, at its own width again.
      await setLayout(page, { rail: false, find: false });
      await settle(page, 300);
      const back = await box(page, TOAST);
      expect(back.top).toBeCloseTo(44, 0);
      expect(back.right).toBeCloseTo(1424, 0);
      expect(back.right - back.left).toBeGreaterThanOrEqual(280);
    } finally {
      await page.close();
    }
  });

  it('over a card that fills the rail the toast stays at the rail top, in the column (1440)', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1440);
    try {
      await setLayout(page, { rail: true, tall: true });
      await showToast(page);
      await settle(page);
      const card = await box(page, CARD);
      const conversation = await box(page, '#cell');
      expect(card.bottom).toBeCloseTo(888, 0);
      const toast = await box(page, TOAST);
      expect(toast.top).toBeCloseTo(44, 0);
      expect(toast.left).toBeCloseTo(card.left, 0);
      expect(toast.right).toBeCloseTo(card.right, 0);
      expect(overlaps(toast, conversation), 'the toast stays off the conversation').toBe(false);
    } finally {
      await page.close();
    }
  });

  it('beside the summary popover the toast sits 16px left of it (1300)', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1300);
    try {
      await showToast(page);
      await settle(page);
      await setLayout(page, { popover: true });
      await settle(page, 300);
      const popover = await box(page, '.br-summary-popover');
      const toast = await box(page, TOAST);
      expect(toast.right).toBeCloseTo(popover.left - 16, 0);
      expect(toast.top).toBeCloseTo(44, 0);
      expect(overlaps(toast, popover)).toBe(false);

      await setLayout(page, { popover: false });
      await settle(page, 300);
      expect((await box(page, TOAST)).right).toBeCloseTo(1284, 0);
    } finally {
      await page.close();
    }
  });

  it('find open in the left pane of a split moves nothing', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1300);
    try {
      await setLayout(page, { split: true, find: true });
      await page.waitForSelector(FIND);
      await showToast(page);
      await settle(page, 300);
      const bar = await box(page, FIND);
      const toast = await box(page, TOAST);
      expect(bar.right).toBeLessThan(700);
      expect(toast.top).toBeCloseTo(44, 0);
      expect(toast.right).toBeCloseTo(1284, 0);
    } finally {
      await page.close();
    }
  });

  for (const how of ['while the window resizes', 'under reduced motion'] as const) {
    it(`a move snaps ${how}`, async (ctx) => {
      if (!browser) return ctx.skip();
      const page = await open(1300, {
        motion: how === 'under reduced motion' ? 'reduce' : 'no-preference',
      });
      try {
        await showToast(page);
        await settle(page);
        if (how === 'while the window resizes') {
          await page.evaluate(() => document.body.classList.add('biorouter-window-resizing'));
        }
        const frames = await watch(page, 'find-open', 250);
        expect(frames.filter((f) => f.gliding)).toEqual([]);
        await settle(page);
        const bar = await box(page, FIND);
        expect((await box(page, TOAST)).top).toBeCloseTo(bar.bottom + 8, 0);
      } finally {
        await page.close();
      }
    });
  }
});
