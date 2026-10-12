// @vitest-environment node
/**
 * An open Select stands above a standing toast, portalled or inside a dialog (WS-PRIMITIVES-2).
 *
 * The toast layer drops one step under the menus while a menu or popover is open ("toast yields
 * to an open menu" in main.css, V1-R7-1). The shared Select names its open react-select menu
 * `data-slot="select-content"`, so a PORTALLED Select (the cron picker) is reached by that rule.
 * A Select that keeps its menu inside a dialog is not: the dialog box is `fixed`, at `--z-modal`
 * and transformed, so the menu's 500 counts only inside it and the whole dialog competes at 400
 * in the page, under the yielded toast (499). The second rule puts the toast behind the dialog
 * while such a menu is open.
 *
 * jsdom has no hit testing and does not evaluate `:has()`, so the real Select and the real
 * react-toastify container are bundled with esbuild, `styles/main.css` is compiled by Tailwind
 * (the vendored ReactToastify.css after it, as App.tsx imports them), and Chromium answers
 * `elementFromPoint`.
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
 * A 600px window. A toast sits in the corner (420 wide, App's layer). `#portalled` mounts a
 * Select whose menu is portalled to <body> and opens under the toast; `#inline` mounts one inside
 * a dialog-shaped box (`.biorouter-modal-surface`, fixed, `--z-modal`, transformed, as
 * `ui/dialog.tsx` draws it) whose menu also opens under the toast.
 */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ToastContainer, toast } from 'react-toastify';
import { Select } from '../ui/Select';
import { NotificationContent, TOAST_SURFACE_CLASS_NAME } from './NotificationSurface';

const options = Array.from({ length: 6 }, (_, i) => ({ value: 'v' + i, label: 'Option ' + i }));

function Harness() {
  const [mode, setMode] = React.useState<'none' | 'portalled' | 'inline'>('none');
  (window as unknown as Record<string, unknown>).__mode = setMode;
  return (
    <>
      {mode === 'portalled' && (
        <div style={{ position: 'absolute', top: 40, right: 24, width: 380 }}>
          <Select
            options={options}
            menuIsOpen
            menuPortalTarget={document.body}
            classNamePrefix="probe"
          />
        </div>
      )}
      {mode === 'inline' && (
        <div
          className="biorouter-modal-surface fixed z-[var(--z-modal)] bg-background-default"
          style={{ top: 30, right: 20, width: 420, height: 400, transform: 'translate(0, 0)' }}
        >
          <div style={{ padding: 10 }}>
            <Select options={options} menuIsOpen classNamePrefix="probe" />
          </div>
        </div>
      )}
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
      message="Failed: CDWAgent, and a second line long enough to make the toast wide"
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

describe('an open Select stands above a standing toast (WS-PRIMITIVES-2)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the toast and Select stacking tests.');
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
      readFileSync(requireFromDesktop.resolve('react-toastify/dist/ReactToastify.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

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

  async function open(): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width: 1048, height: 600 } });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.setContent(
      `<!doctype html><html class="light"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: script });
    await page.evaluate(() => (window as unknown as { __toast: () => void }).__toast());
    await page.waitForSelector('[data-testid="toast-action"]');
    await settle(page);
    return page;
  }

  const setMode = (page: Page, mode: 'none' | 'portalled' | 'inline') =>
    page.evaluate((m) => (window as unknown as { __mode: (m: string) => void }).__mode(m), mode);

  /** For each option under the toast: does its centre hit the option? */
  const optionsUnderToast = (page: Page) =>
    page.evaluate((toastSelector) => {
      const toast = document.querySelector(toastSelector)!.getBoundingClientRect();
      return [...document.querySelectorAll('[data-slot="select-content"] .probe__option')]
        .map((el) => {
          const r = el.getBoundingClientRect();
          const x = r.left + r.width / 2;
          const y = r.top + r.height / 2;
          const hit = document.elementFromPoint(x, y);
          return {
            text: el.textContent ?? '',
            under: x > toast.left && x < toast.right && y > toast.top && y < toast.bottom,
            itself: !!hit && (hit === el || el.contains(hit)),
            hit: hit?.outerHTML.slice(0, 80),
          };
        })
        .filter((o) => o.under);
    }, TOAST);

  const toastZ = (page: Page) =>
    page.evaluate(() =>
      Number(getComputedStyle(document.querySelector('.Toastify__toast-container')!).zIndex)
    );

  for (const mode of ['portalled', 'inline'] as const) {
    const where = mode === 'inline' ? 'inside a dialog' : 'portalled to the page';
    it(`the options of a Select ${where} under the toast take the pointer`, async (ctx) => {
      if (!browser) return ctx.skip();
      const page = await open();
      try {
        expect(await toastZ(page), 'a toast on its own is at --z-toast').toBe(600);
        await setMode(page, mode);
        await page.waitForSelector('[data-slot="select-content"][data-state="open"]');
        await settle(page);
        const under = await optionsUnderToast(page);
        expect(under.length, 'some options open under the toast').toBeGreaterThan(0);
        for (const option of under) {
          expect(option.itself, `"${option.text}" hit ${option.hit}`).toBe(true);
        }
        expect(await toastZ(page)).toBe(mode === 'portalled' ? 499 : 399);

        // Closed, the toast is back on top and its action takes the click again.
        await setMode(page, 'none');
        await settle(page);
        expect(await toastZ(page)).toBe(600);
        const action = await page.evaluate(() => {
          const el = document.querySelector('[data-testid="toast-action"]')!;
          const r = el.getBoundingClientRect();
          const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
          return !!hit && (hit === el || el.contains(hit));
        });
        expect(action).toBe(true);
      } finally {
        await page.close();
      }
    });
  }
});
