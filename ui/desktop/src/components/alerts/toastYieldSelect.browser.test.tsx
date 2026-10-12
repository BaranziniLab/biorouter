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
 *
 * Each Select is mounted closed and opened after its first commit (`OpenedSelect`). Mounted
 * already open, the portalled menu never appeared on Linux: on CI (ubuntu, Google Chrome stable,
 * a7d069304) the portalled case ran into vitest's bare "Test timed out in 30000ms", and in a
 * Linux container with Playwright's Chromium it failed on every run, while macOS passed in under
 * a second (see `OpenedSelect` for why). The app is not affected: no Select in it mounts open.
 *
 * Every wait in a case is bounded well under the case's 30 s budget and named (`bounded`,
 * `waitFor`, `settle`). A Playwright wait's own default (30 s) equals the case budget, and
 * `page.evaluate`, `newPage` and `close` have no timeout at all, so on CI the wait that never
 * returned could not say which it was. Now the first one that hangs fails the case with its name
 * and a report of the page: visibility and focus, whether a frame still comes, the toast and the
 * Select menu in the DOM, and every finite animation that has not finished. The assertions are
 * unchanged.
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

// Mounted closed and opened after its first commit, as a person opens one. react-select hands
// its menu portal the control from a ref, so a Select mounted already open gives the portal null
// on that render and the portalled menu never appears unless something renders it again. On
// macOS something does (componentDidMount records an Apple device in state); on Linux and
// Windows nothing does.
function OpenedSelect(props: React.ComponentProps<typeof Select>) {
  const [open, setOpen] = React.useState(false);
  React.useEffect(() => setOpen(true), []);
  return <Select {...props} menuIsOpen={open} />;
}

function Harness() {
  const [mode, setMode] = React.useState<'none' | 'portalled' | 'inline'>('none');
  (window as unknown as Record<string, unknown>).__mode = setMode;
  return (
    <>
      {mode === 'portalled' && (
        <div style={{ position: 'absolute', top: 40, right: 24, width: 380 }}>
          <OpenedSelect
            options={options}
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
            <OpenedSelect options={options} classNamePrefix="probe" />
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

/** A step's bound: one stuck step fails the case with its name long before the 30 s budget. */
const STEP_MS = 10_000;
/** `settle`'s own bound, inside the page; the node side allows the report on top of it. */
const SETTLE_MS = 5_000;
/** How long the page may take to report on itself after a step failed. */
const REPORT_MS = 3_000;

/**
 * What the page is doing, for a step that failed: visibility and focus, whether a frame still
 * comes within a second, the toast and Select menu in the DOM, and each finite animation that has
 * not finished (name, play state, target).
 */
const pageReport = (page: Page) =>
  page.evaluate(
    (toastSelector) =>
      new Promise<string>((done) => {
        let frame = 'no frame came in 1 s';
        requestAnimationFrame(() => (frame = 'a frame came'));
        setTimeout(() => {
          const animations = document
            .getAnimations()
            .filter(
              (a) =>
                a.playState !== 'finished' &&
                Number.isFinite(a.effect?.getComputedTiming().endTime ?? 0)
            )
            .map((a) => {
              const kind =
                'animationName' in a
                  ? `animation ${String(a.animationName)}`
                  : 'transitionProperty' in a
                    ? `transition ${String(a.transitionProperty)}`
                    : `script animation ${a.id || '(no id)'}`;
              const target = (a.effect as KeyframeEffect | null)?.target;
              const classes = (target?.getAttribute('class') ?? '').trim().split(/\s+/);
              const where = target
                ? [target.tagName.toLowerCase(), ...classes.filter(Boolean)].join('.').slice(0, 90)
                : 'no target';
              return `${kind} on ${where} (${a.playState}${a.pending ? ', pending' : ''})`;
            });
          const menus = [...document.querySelectorAll('[data-slot="select-content"]')].map(
            (el) => el.getAttribute('data-state') ?? 'no state'
          );
          done(
            [
              `page ${document.visibilityState}${document.hasFocus() ? ', focused' : ', not focused'}`,
              frame,
              `${document.querySelectorAll(toastSelector).length} toast(s)`,
              `toast action ${document.querySelector('[data-testid="toast-action"]') ? 'present' : 'absent'}`,
              `Select menus [${menus.join(', ')}]`,
              `unfinished animations: ${animations.join('; ') || 'none'}`,
            ].join('; ')
          );
        }, 1_000);
      }),
    TOAST
  );

/** `pageReport`, bounded: a page whose main thread is stuck cannot answer at all. */
async function report(page: Page | null): Promise<string> {
  if (!page) return '';
  const answer = pageReport(page);
  answer.catch(() => undefined);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const silent = new Promise<string>((resolve) => {
    timer = setTimeout(
      () => resolve(`the page did not answer in ${REPORT_MS} ms (its main thread is busy or hung)`),
      REPORT_MS
    );
  });
  try {
    return `. Page: ${await Promise.race([answer, silent])}`;
  } catch (error) {
    return `. The page could not report: ${error instanceof Error ? error.message : String(error)}`;
  } finally {
    clearTimeout(timer);
  }
}

const EXPIRED = Symbol('expired');

/**
 * One named step of a case. It fails with its name and a report of `page` when `work` rejects
 * or has not finished in `ms`. An abandoned step's late rejection (its page closed under it) is
 * swallowed, so it cannot surface as an unhandled error in a later case.
 */
async function bounded<T>(
  page: Page | null,
  label: string,
  ms: number,
  work: () => Promise<T>
): Promise<T> {
  const running = work();
  running.catch(() => undefined);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<typeof EXPIRED>((resolve) => {
    timer = setTimeout(() => resolve(EXPIRED), ms);
  });
  let outcome: T | typeof EXPIRED;
  try {
    outcome = await Promise.race([running, expired]);
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    throw new Error(`${label} failed: ${reason}${await report(page)}`);
  } finally {
    clearTimeout(timer);
  }
  if (outcome === EXPIRED) {
    throw new Error(`${label} did not finish in ${ms} ms${await report(page)}`);
  }
  return outcome;
}

/** `page.waitForSelector` with its own timeout under the step bound, named. */
const waitFor = (page: Page, label: string, selector: string) =>
  bounded(page, `waiting for ${label} (${selector})`, STEP_MS + 2_000, () =>
    page.waitForSelector(selector, { timeout: STEP_MS })
  );

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

  /**
   * Two frames, every finite animation finished, and a pause. Bounded inside the page at
   * SETTLE_MS, failing with how many of the two frames came (`bounded` adds the page report,
   * with the animations still unfinished), and again from here, for a page that cannot run its
   * own timer.
   */
  const settle = (page: Page, label: string, ms = 120) =>
    bounded(page, `settling ${label}`, SETTLE_MS + REPORT_MS, () =>
      page.evaluate(
        ({ wait, limit }) =>
          new Promise<void>((done, fail) => {
            let frames = 0;
            const timer = setTimeout(
              () => fail(new Error(`did not settle in ${limit} ms: ${frames} of 2 frames came`)),
              limit
            );
            requestAnimationFrame(() => {
              frames = 1;
              requestAnimationFrame(async () => {
                frames = 2;
                await Promise.all(
                  document
                    .getAnimations()
                    .filter((a) => Number.isFinite(a.effect?.getComputedTiming().endTime ?? 0))
                    .map((a) => a.finished.catch(() => undefined))
                );
                setTimeout(() => {
                  clearTimeout(timer);
                  done();
                }, wait);
              });
            });
          }),
        { wait: ms, limit: SETTLE_MS }
      )
    );

  async function open(): Promise<Page> {
    const page = await bounded(null, 'opening a page', STEP_MS, () =>
      browser!.newPage({ viewport: { width: 1048, height: 600 } })
    );
    try {
      await bounded(page, 'asking for reduced motion', STEP_MS, () =>
        page.emulateMedia({ reducedMotion: 'reduce' })
      );
      await bounded(page, 'writing the harness page', STEP_MS + 2_000, () =>
        page.setContent(
          `<!doctype html><html class="light"><head><style>${css}</style></head>` +
            `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`,
          { timeout: STEP_MS }
        )
      );
      await bounded(page, 'running the harness bundle', STEP_MS + 2_000, () =>
        page.addScriptTag({ content: script })
      );
      await bounded(page, 'raising the toast', STEP_MS, () =>
        page.evaluate(() => (window as unknown as { __toast: () => void }).__toast())
      );
      await waitFor(page, 'the toast action', '[data-testid="toast-action"]');
      await settle(page, 'after the toast');
      return page;
    } catch (error) {
      await closePage(page).catch(() => undefined);
      throw error;
    }
  }

  const closePage = (page: Page) => bounded(null, 'closing the page', STEP_MS, () => page.close());

  const setMode = (page: Page, mode: 'none' | 'portalled' | 'inline') =>
    bounded(page, `setting the harness to ${mode}`, STEP_MS, () =>
      page.evaluate((m) => (window as unknown as { __mode: (m: string) => void }).__mode(m), mode)
    );

  /** For each option under the toast: does its centre hit the option? */
  const optionsUnderToast = (page: Page) =>
    bounded(page, 'hit testing the options', STEP_MS, () =>
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
      }, TOAST)
    );

  const toastZ = (page: Page) =>
    bounded(page, 'reading the toast layer', STEP_MS, () =>
      page.evaluate(() =>
        Number(getComputedStyle(document.querySelector('.Toastify__toast-container')!).zIndex)
      )
    );

  for (const mode of ['portalled', 'inline'] as const) {
    const where = mode === 'inline' ? 'inside a dialog' : 'portalled to the page';
    it(`the options of a Select ${where} under the toast take the pointer`, async (ctx) => {
      if (!browser) return ctx.skip();
      const page = await open();
      let passed = false;
      try {
        expect(await toastZ(page), 'a toast on its own is at --z-toast').toBe(600);
        await setMode(page, mode);
        await waitFor(
          page,
          'the open Select menu',
          '[data-slot="select-content"][data-state="open"]'
        );
        await settle(page, 'with the menu open');
        const under = await optionsUnderToast(page);
        expect(under.length, 'some options open under the toast').toBeGreaterThan(0);
        for (const option of under) {
          expect(option.itself, `"${option.text}" hit ${option.hit}`).toBe(true);
        }
        expect(await toastZ(page)).toBe(mode === 'portalled' ? 499 : 399);

        // Closed, the toast is back on top and its action takes the click again.
        await setMode(page, 'none');
        await settle(page, 'with the menu closed');
        expect(await toastZ(page)).toBe(600);
        const action = await bounded(page, 'hit testing the toast action', STEP_MS, () =>
          page.evaluate(() => {
            const el = document.querySelector('[data-testid="toast-action"]')!;
            const r = el.getBoundingClientRect();
            const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
            return !!hit && (hit === el || el.contains(hit));
          })
        );
        expect(action).toBe(true);
        passed = true;
      } finally {
        // A close that hangs is named too, but never hides the error that ended the case.
        await closePage(page).catch((error) => {
          if (passed) throw error;
        });
      }
    });
  }
});
