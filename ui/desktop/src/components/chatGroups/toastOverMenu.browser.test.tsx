// @vitest-environment node
/**
 * A menu the person opens stands above a toast that is still showing (vision V1-R7-1).
 *
 * The extension-load report is sticky, and it docks in the top-right corner, exactly where the
 * tab strip's ▾ menu and a tab's right-click menu open. Menus sit at `--z-modal-dropdown` (500)
 * and the toast layer at `--z-toast` (600), so in the running app the centre of the ▾ menu's
 * filter field and the menu's "Open in new window" and "Copy chat ID" hit-tested to the toast,
 * and no pointer could reach them. `main.css` drops the toast layer one step below the menus
 * while one is open ("toast yields to an open menu").
 *
 * jsdom has no layout and no hit testing and does not evaluate `:has()`, so the real strip and
 * the real react-toastify container are bundled with esbuild, `styles/main.css` is compiled by
 * Tailwind (with the vendored ReactToastify.css after it, the order App.tsx imports them in),
 * and Chromium answers `elementFromPoint`.
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
 * A chat band the width of the window with 14 tabs (so the ▾ and its filter show), and the toast
 * layer as App.tsx configures it, holding one sticky report shaped like the extension-load one.
 */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ToastContainer, toast } from 'react-toastify';
import { ChatTabStrip } from './ChatTabStrip';
import { NotificationContent, TOAST_SURFACE_CLASS_NAME } from '../alerts/NotificationSurface';

const tabs = Array.from({ length: 14 }, (_, i) => ({
  tabId: 'tab-' + i,
  sessionId: 's' + i,
  title: 'A fairly long chat title ' + i,
  userSetName: false,
}));

function Harness() {
  const [active, setActive] = React.useState('tab-13');
  return (
    <>
      <div id="band" style={{ display: 'flex', height: 44, width: '100%' }}>
        <ChatTabStrip
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
      <ToastContainer
        aria-label="Toast notifications"
        toastClassName={() => TOAST_SURFACE_CLASS_NAME}
        style={{
          '--toastify-z-index': 'var(--z-toast)',
          top: 'var(--toast-inset-top)',
          width: 'fit-content',
          minWidth: '280px',
          maxWidth: 'min(420px, calc(100vw - 32px))',
        } as React.CSSProperties}
        position="top-right"
        newestOnTop={false}
        closeOnClick={false}
        hideProgressBar
        icon={false}
      />
    </>
  );
}

createRoot(document.getElementById('root')!).render(<Harness />);
(window as unknown as Record<string, unknown>).__toast = () =>
  toast(
    <NotificationContent
      status="error"
      title="1 of 2 extensions loaded"
      message="Failed: CDWAgent"
      actions={<button type="button" data-testid="toast-action">View details</button>}
    />,
    { autoClose: false, toastId: 'extension-loading' }
  );
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

const TOAST = '.Toastify__toast-container > *';

describe('a menu opened under a standing toast can be reached (V1-R7-1)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the toast and menu stacking tests.');
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
      readFileSync(resolve(here, 'chat-tabs.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(theme: 'light' | 'dark' = 'light'): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width: 1440, height: 600 } });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.setContent(
      `<!doctype html><html class="${theme}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: script });
    await page.waitForSelector('[data-testid="chat-tab-overflow-trigger"]');
    await page.evaluate(() => (window as unknown as { __toast: () => void }).__toast());
    await page.waitForSelector('[data-testid="toast-action"]');
    await settle(page);
    return page;
  }

  /** Two frames, every finite animation finished, and a pause. */
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

  /** The box of the first element matching a selector. */
  const box = (page: Page, selector: string) =>
    page.evaluate((s) => {
      const r = document.querySelector(s)!.getBoundingClientRect();
      return { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
    }, selector);

  /** Does the element at the centre of `selector`'s box belong to it? */
  const centreHitsItself = (page: Page, selector: string) =>
    page.evaluate((s) => {
      const el = document.querySelector(s)!;
      const r = el.getBoundingClientRect();
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      return {
        itself: !!hit && (hit === el || el.contains(hit)),
        hit: hit?.outerHTML.slice(0, 80),
      };
    }, selector);

  const overlaps = (
    a: { left: number; top: number; right: number; bottom: number },
    b: { left: number; top: number; right: number; bottom: number }
  ) => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

  const toastZ = (page: Page) =>
    page.evaluate(() =>
      Number(getComputedStyle(document.querySelector('.Toastify__toast-container')!).zIndex)
    );

  for (const theme of ['light', 'dark'] as const) {
    it(`the ▾ menu's filter field is reachable under the toast (${theme})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const page = await open(theme);
      try {
        expect(await toastZ(page), 'a toast on its own is at --z-toast').toBe(600);
        expect((await centreHitsItself(page, TOAST)).itself, 'and on top').toBe(true);

        await page.click('[data-testid="chat-tab-overflow-trigger"]');
        await page.waitForSelector('[data-testid="chat-tab-overflow-menu"] input');
        await settle(page);
        const field = '[data-testid="chat-tab-overflow-menu"] input';
        expect(
          overlaps(await box(page, field), await box(page, TOAST)),
          'the filter field opens under the toast, as in the running app'
        ).toBe(true);
        const hit = await centreHitsItself(page, field);
        expect(hit.itself, `the field's centre hit ${hit.hit}`).toBe(true);
        expect(await toastZ(page), 'still above every dialog (--z-modal, 400)').toBe(499);

        // Filter by typing, then click a row under the toast: it is the menu that answers.
        await page.keyboard.type('title 3');
        await settle(page);
        const row = '[data-testid="chat-tab-overflow-item-tab-3"]';
        expect((await centreHitsItself(page, row)).itself).toBe(true);

        // Closed, the toast is back on top and its action takes the click again.
        await page.keyboard.press('Escape');
        await page.waitForSelector('[data-testid="chat-tab-overflow-menu"]', { state: 'detached' });
        await settle(page);
        expect(await toastZ(page)).toBe(600);
        expect((await centreHitsItself(page, '[data-testid="toast-action"]')).itself).toBe(true);
      } finally {
        await page.close();
      }
    });
  }

  it("a tab's right-click menu near the corner is reachable under the toast", async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open();
    try {
      // The last tab clear of the end, right-clicked near its right end: the menu opens at the
      // cursor and runs under the toast.
      const target = await page.evaluate(() => {
        const strip = document.querySelector('[role="tablist"]')!.getBoundingClientRect();
        const tabs = [...document.querySelectorAll<HTMLElement>('[data-tab-id] [role="tab"]')];
        const shown = tabs.filter((t) => {
          const r = t.getBoundingClientRect();
          return r.left >= strip.left && r.right <= strip.right - 40;
        });
        const r = shown[shown.length - 1].getBoundingClientRect();
        return { x: r.right - 10, y: r.top + r.height / 2 };
      });
      await page.mouse.click(target.x, target.y, { button: 'right' });
      await page.waitForSelector('[data-slot="context-menu-content"] [role="menuitem"]');
      await settle(page);
      const items = await page.evaluate((toastSelector) => {
        const toast = document.querySelector(toastSelector)!.getBoundingClientRect();
        return [
          ...document.querySelectorAll('[data-slot="context-menu-content"] [role="menuitem"]'),
        ].map((el) => {
          const r = el.getBoundingClientRect();
          const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
          return {
            text: el.textContent ?? '',
            underToast:
              r.left < toast.right &&
              toast.left < r.right &&
              r.top < toast.bottom &&
              toast.top < r.bottom,
            itself: !!hit && (hit === el || el.contains(hit)),
            hit: hit?.outerHTML.slice(0, 80),
          };
        });
      }, TOAST);
      expect(items.length).toBeGreaterThan(3);
      for (const item of items) {
        expect(item.itself, `"${item.text}" hit ${item.hit}`).toBe(true);
      }
      const underToast = items.filter((item) => item.underToast).length;
      expect(underToast, 'some of the menu runs under the toast').toBeGreaterThan(0);
    } finally {
      await page.close();
    }
  });
});
