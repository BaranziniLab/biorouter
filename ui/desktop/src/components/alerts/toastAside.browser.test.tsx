// @vitest-environment node
/**
 * A toast in the corner never covers an action on the page under it (final smoke, 2026-10-12).
 *
 * On the Extensions page at 1440 the sticky "1 of 2 extensions loaded" report sat at 1144,44 to
 * 1424,146, over the load-failure notice's actions: the right 32px of "Copy error" (1086 to 1176)
 * and all of the dismiss mark. The stack has nowhere to go there that covers nothing (under the
 * notice is the list's first switch, left of the actions is the error), so the notice's actions
 * step left, out from under the stack, on the title's row and no closer to the title than 8px
 * (`toastClearance.ts`, `data-toast-aside`). Nothing else on the page moves or rewraps, and the
 * error keeps wrapping before the actions' column, clear of the stack.
 *
 * jsdom has no layout, so the real notice inside the real reading column (`ReadableContent`,
 * `chat` size, `px-6`, under a 44px band beside a 288px sidebar, as `ExtensionsView` mounts it),
 * the real extension-load report, the real react-toastify container as App.tsx configures it and
 * the watcher App.tsx starts are bundled with esbuild, `styles/main.css` is compiled by Tailwind
 * (the vendored ReactToastify.css after it, as App imports them, and the notice's own sheet), and
 * Chromium measures.
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

const ERROR =
  "invalid config: Failed to fetch secret 'CLINICAL_RECORDS_USERNAME' from config: " +
  'Configuration value not found: CLINICAL_RECORDS_USERNAME';

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { ToastContainer, toast } from 'react-toastify';
import { ReadableContent } from '../Layout/ReadableContent';
import { ExtensionLoadFailureNotice } from '../extensions/ExtensionLoadFailureNotice';
import { GroupedExtensionLoadingToast } from '../GroupedExtensionLoadingToast';
import { recordExtensionLoadResults } from '../../utils/extensionLoadFailures';
import { TOAST_SURFACE_CLASS_NAME } from './NotificationSurface';
import { watchToastClearance } from './toastClearance';

recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: ${JSON.stringify(ERROR)} }]);

function Harness() {
  React.useEffect(() => watchToastClearance(), []);
  return (
    <MemoryRouter>
      <div style={{ position: 'fixed', inset: 0, display: 'flex' }}>
        <div style={{ width: 288, flex: 'none' }} />
        <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
          <div style={{ height: 44, flex: 'none' }} />
          <ReadableContent size="chat" className="relative min-h-0 flex-1 px-6">
            <div className="relative py-4">
              <ExtensionLoadFailureNotice onAskBiorouter={() => {}} />
              <div id="first-row" style={{ height: 40 }}>Crew</div>
            </div>
          </ReadableContent>
        </div>
      </div>
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
    </MemoryRouter>
  );
}

createRoot(document.getElementById('root')!).render(<Harness />);
const w = window as unknown as Record<string, unknown>;
w.__toast = () =>
  toast(
    <GroupedExtensionLoadingToast
      extensions={[
        { name: 'crew', status: 'success' },
        { name: 'cdwagent', status: 'error', error: 'invalid config' },
      ]}
      totalCount={2}
      isComplete
    />,
    { autoClose: false, toastId: 'extension-load' }
  );
w.__dismiss = () => toast.dismiss('extension-load');
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
type Action = Box & { name: string; reachable: boolean; hit: string };

const TOAST = '.Toastify__toast-container > *';
const NOTICE = '[data-testid="extension-load-failures"] [role="alert"]';

const overlaps = (a: Box, b: Box) =>
  a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

describe('a toast in the corner leaves the actions of the page under it reachable', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the toast and in-page action tests.');
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
      readFileSync(resolve(here, '../extensions/extension-notice.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(width: number): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width, height: 900 } });
    await page.setContent(
      `<!doctype html><html class="light"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: script });
    await page.waitForSelector(NOTICE);
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

  const call = (page: Page, fn: '__toast' | '__dismiss') =>
    page.evaluate((name) => (window as unknown as Record<string, () => void>)[name](), fn);

  const box = (page: Page, selector: string): Promise<Box> =>
    page.evaluate((s) => {
      const r = document.querySelector(s)!.getBoundingClientRect();
      return { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
    }, selector);

  /** Each of the notice's buttons: its box, and whether its centre and both ends take the pointer. */
  const actions = (page: Page): Promise<Action[]> =>
    page.evaluate((notice) => {
      return [...document.querySelectorAll(`${notice} button`)].map((el) => {
        const r = el.getBoundingClientRect();
        const y = r.top + r.height / 2;
        const probes = [r.left + 2, r.left + r.width / 2, r.right - 2].map((x) =>
          document.elementFromPoint(x, y)
        );
        const miss = probes.find((hit) => !hit || !(hit === el || el.contains(hit)));
        // The shown label only: Copy error keeps "Copied" in its cell, hidden, to hold its width.
        const shown = [...el.querySelectorAll('span')].find(
          (s) => !s.closest('[aria-hidden="true"]') && s.children.length === 0
        );
        return {
          name: el.getAttribute('aria-label') ?? shown?.textContent ?? el.textContent ?? '',
          left: r.left,
          top: r.top,
          right: r.right,
          bottom: r.bottom,
          reachable: miss === undefined,
          hit: miss ? miss.outerHTML.slice(0, 80) : '',
        };
      });
    }, NOTICE);

  it('at rest the notice reads as the Extensions page draws it, its actions at the trailing edge (1440)', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1440);
    try {
      await settle(page);
      const notice = await box(page, NOTICE);
      // The page's geometry in the final smoke's screenshot: the reading column's 712px inside.
      expect(notice.left).toBeCloseTo(508, 0);
      expect(notice.top).toBeCloseTo(60, 0);
      expect(notice.right).toBeCloseTo(1220, 0);
      const all = await actions(page);
      expect(all.map((a) => a.name)).toEqual(['Ask Biorouter', 'Copy error', 'Dismiss']);
      // The dismiss mark ends at the note's 12px inner edge, and the error wraps 8px before the
      // actions' column, as it did beside the Note's action slot.
      expect(all[2].right).toBeCloseTo(notice.right - 12, 0);
      const error = await box(page, `${NOTICE} .br-ext-notice-error`);
      expect(error.right).toBeCloseTo(all[0].left - 8, 0);
      for (const a of all) expect(a.reachable, `${a.name} hit ${a.hit}`).toBe(true);
    } finally {
      await page.close();
    }
  });

  it('a sticky toast arrives: every action of the notice stays clear of it, and nothing below moves (1440)', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1440);
    try {
      await settle(page);
      const noticeBefore = await box(page, NOTICE);
      const rowBefore = await box(page, '#first-row');

      await call(page, '__toast');
      await page.waitForSelector(TOAST);
      await settle(page, 300);
      const toast = await box(page, TOAST);
      // The stack keeps its corner: nothing in the corner is a surface it steps around.
      expect(toast.top).toBeCloseTo(44, 0);
      expect(toast.right).toBeCloseTo(1424, 0);
      expect(overlaps(toast, noticeBefore), 'the toast does sit over the notice').toBe(true);

      for (const a of await actions(page)) {
        expect(a.reachable, `${a.name} hit ${a.hit}`).toBe(true);
        expect(overlaps(a, toast), `${a.name} under the toast`).toBe(false);
        // 16px clear of the stack, the gap it keeps beside the summary popover.
        expect(a.right).toBeLessThanOrEqual(toast.left - 16 + 0.5);
      }
      // The notice, its height and the row under it stay where they were: only the actions moved.
      expect(await box(page, NOTICE)).toEqual(noticeBefore);
      expect(await box(page, '#first-row')).toEqual(rowBefore);
      // The name and the error stay clear of the stack too: the error wraps before the actions'
      // column, as it did in the Note's action slot, and the actions stop short of the name.
      const name = await box(page, `${NOTICE} .br-ext-notice-name`);
      const error = await box(page, `${NOTICE} .br-ext-notice-error`);
      expect(overlaps(error, toast), 'the error under the toast').toBe(false);
      expect(overlaps(name, toast), 'the name under the toast').toBe(false);
      const [first] = await actions(page);
      expect(first.left).toBeGreaterThanOrEqual(name.right + 8 - 0.5);
    } finally {
      await page.close();
    }
  });

  it('the toast goes: the actions return to the trailing edge, but not from under a pointer resting on them', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1440);
    try {
      await settle(page);
      const rest = await actions(page);
      await call(page, '__toast');
      await page.waitForSelector(TOAST);
      await settle(page, 300);
      const stepped = await actions(page);
      expect(stepped[1].right).toBeLessThan(rest[1].right - 1);

      // A pointer on Copy error holds the actions where they are when the toast goes.
      const copy = stepped[1];
      await page.mouse.move((copy.left + copy.right) / 2, (copy.top + copy.bottom) / 2);
      await call(page, '__dismiss');
      await page.waitForSelector(TOAST, { state: 'detached' });
      await settle(page, 300);
      expect((await actions(page))[1]).toMatchObject({ left: copy.left, right: copy.right });

      // Away from them, they go back to the trailing edge.
      await page.mouse.move(400, 600);
      await settle(page, 300);
      const back = await actions(page);
      for (let i = 0; i < rest.length; i++) {
        expect(back[i].left).toBeCloseTo(rest[i].left, 0);
        expect(back[i].right).toBeCloseTo(rest[i].right, 0);
      }
    } finally {
      await page.close();
    }
  });

  it('at the narrowest window the actions stay reachable beside a standing toast (1048)', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1048);
    try {
      await call(page, '__toast');
      await page.waitForSelector(TOAST);
      await settle(page, 300);
      const toast = await box(page, TOAST);
      for (const a of await actions(page)) {
        expect(a.reachable, `${a.name} hit ${a.hit}`).toBe(true);
        expect(overlaps(a, toast), `${a.name} under the toast`).toBe(false);
      }
    } finally {
      await page.close();
    }
  });
});
