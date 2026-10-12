// @vitest-environment node
/**
 * Keyboard focus survives "Check for updates", in a real browser (vision round 3, V4-R3-4; the
 * same class as V3-R2-1).
 *
 * jsdom cannot see this defect: it keeps focus on a button that turns `disabled`, while Chromium
 * hands focus to `<body>` the moment a focused button does. The About list's Check for updates was
 * `disabled` while a check or download ran, so a keyboard user who pressed Enter on it lost their
 * place in Settings and had to Tab in from the top. It is `aria-busy` now: still enabled, a press
 * ignored, dimmed by `.br-action-busy`.
 *
 * The REAL `UpdateSection` is bundled with esbuild and rendered by React in Chromium; only
 * `window.electron` is a stub, with a check that takes 400ms so there is a window to sample. The
 * case is read three times: while the check is out, when the verdict lands, and through a
 * download into "Restart to update", where the label changes in the same element.
 *
 * "The instrument" proves the case can fail: the same bundle with the new attributes swapped back
 * for the old `disabled={busy}` must leave focus on BODY. A mutation that matches nothing throws,
 * so a refactor cannot turn the check into a no-op.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../../styles/main.css'), 'utf8');

/** The `@ws WS-VIEWS-A begin: busy action` block, the only stylesheet this case needs. */
function busyActionRule(): string {
  const match = MAIN_CSS.match(
    /\/\* @ws WS-VIEWS-A begin: busy action \*\/([\s\S]*?)\/\* @ws WS-VIEWS-A end: busy action \*\//
  );
  if (!match) throw new Error('main.css has no `@ws WS-VIEWS-A begin: busy action` block');
  return match[1];
}

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import UpdateSection from './UpdateSection';

type Probe = {
  delay: number;
  calls: string[];
  emit: (p: { event: string; data?: unknown }) => void;
};
const probe = {
  delay: 400,
  calls: [] as string[],
  emit: () => {},
} as Probe;
(window as unknown as { __upd: Probe }).__upd = probe;

(window as unknown as { electron: unknown }).electron = {
  getVersion: () => '1.92.1',
  onUpdaterEvent: (cb: (p: { event: string; data?: unknown }) => void) => {
    probe.emit = cb;
    return () => {};
  },
  getUpdateState: async () => null,
  checkForUpdates: async () => {
    probe.calls.push('check');
    await new Promise((resolve) => setTimeout(resolve, probe.delay));
    return { updateInfo: {}, error: null };
  },
  installUpdate: () => {},
};

createRoot(document.getElementById('root')!).render(<UpdateSection />);
`;

/** The new attributes swapped back for the old `disabled` attribute. Must match, or throw. */
const OLD_DISABLED: Array<[string, string]> = [
  [
    `aria-disabled={busy || undefined}
        aria-busy={busy || undefined}
        onClick={busy ? undefined : checkForUpdates}`,
    `disabled={busy}
        onClick={checkForUpdates}`,
  ],
];

async function bundle(restoreDisabled: boolean): Promise<string> {
  const mutate: Plugin = {
    name: 'mutate',
    setup(b) {
      if (!restoreDisabled) return;
      b.onLoad({ filter: /\/UpdateSection\.tsx$/ }, (args) => {
        let source = readFileSync(args.path, 'utf8');
        for (const [from, to] of OLD_DISABLED) {
          if (!source.includes(from))
            throw new Error(`mutation target missing in ${args.path}: ${from}`);
          source = source.split(from).join(to);
        }
        return { contents: source, loader: 'tsx', resolveDir: dirname(args.path) };
      });
    },
  };
  const out = await build({
    stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
    bundle: true,
    write: false,
    format: 'iife',
    jsx: 'automatic',
    platform: 'browser',
    define: { 'process.env.NODE_ENV': '"production"' },
    loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
    logLevel: 'silent',
    plugins: [mutate],
  });
  return out.outputFiles[0].text;
}

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

type Probe = { tag: string; label: string | null; marked: boolean };
type Upd = { calls: string[]; emit: (p: { event: string; data?: unknown }) => void };

describe('keyboard focus survives Check for updates (V4-R3-4)', () => {
  let browser: Browser | null = null;
  let fixed = '';
  let old = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the focus cases.');
      return;
    }
    [fixed, old] = await Promise.all([bundle(false), bundle(true)]);
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(script: string): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1100, height: 700 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${busyActionRule()}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    return tab;
  }

  const probe = (tab: Page): Promise<Probe> =>
    tab.evaluate(() => {
      const el = document.activeElement as (HTMLElement & { __probe?: boolean }) | null;
      return {
        tag: el?.tagName ?? 'none',
        label: el?.getAttribute('aria-label') ?? null,
        marked: Boolean(el?.__probe),
      };
    });

  /** Focus the button, mark its DOM node, press Enter, and wait until the check is out. */
  async function pressEnterOnCheck(tab: Page) {
    const control = tab.locator('button[aria-label="Check for updates"]');
    await control.waitFor();
    await control.focus();
    await control.evaluate((node) => {
      (node as HTMLElement & { __probe?: boolean }).__probe = true;
    });
    await tab.keyboard.press('Enter');
    await tab.waitForFunction(() => (window as unknown as { __upd: Upd }).__upd.calls.length > 0);
    // Blink drops the focus of a control that stopped being focusable in a task of its own, not
    // inside the render that disabled it, so a sample taken in the same turn can still see the
    // button focused. The check takes 400ms, so this is still well inside it.
    await tab.waitForTimeout(120);
  }

  const stillTheSameFocusedControl = (p: Probe, label: string) => {
    expect(p.tag, 'focus must not fall to the page').toBe('BUTTON');
    expect(p.marked, 'the focused element must be the one that was pressed').toBe(true);
    expect(p.label).toBe(label);
  };

  it('keeps focus on the button while a check is out and after the verdict lands', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(fixed);
    try {
      await pressEnterOnCheck(tab);
      stillTheSameFocusedControl(await probe(tab), 'Check for updates');

      const during = await tab.evaluate(() => {
        const b = document.querySelector('button[aria-label="Check for updates"]') as HTMLElement;
        return {
          disabled: (b as HTMLButtonElement).disabled,
          busy: b.getAttribute('aria-busy'),
          opacity: getComputedStyle(b).opacity,
        };
      });
      expect(during.disabled, 'busy is advertised, the control is not disabled').toBe(false);
      expect(during.busy).toBe('true');
      expect(during.opacity, '.br-action-busy dims it').toBe('0.5');

      // A second Enter while it is out starts nothing.
      await tab.keyboard.press('Enter');
      await tab.waitForTimeout(40);
      expect(await tab.evaluate(() => (window as unknown as { __upd: Upd }).__upd.calls)).toEqual([
        'check',
      ]);

      // The verdict lands (the main process would emit it; the stub answers after 400ms).
      await tab.evaluate(() =>
        (window as unknown as { __upd: Upd }).__upd.emit({ event: 'update-not-available' })
      );
      await tab.waitForFunction(() => !document.querySelector('[aria-busy="true"]'));
      stillTheSameFocusedControl(await probe(tab), 'Check for updates');
    } finally {
      await tab.close();
    }
  });

  it('keeps focus when the check turns into a download and then Restart to update', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(fixed);
    try {
      await pressEnterOnCheck(tab);
      await tab.evaluate(() => {
        const upd = (window as unknown as { __upd: Upd }).__upd;
        upd.emit({ event: 'update-available', data: { version: '1.93.0' } });
        upd.emit({ event: 'download-progress', data: { percent: 40 } });
      });
      // Attached, not visible: the page loads no stylesheet but the busy rule, so the bar has no box.
      await tab.waitForSelector('[role="progressbar"]', { state: 'attached' });
      stillTheSameFocusedControl(await probe(tab), 'Check for updates');

      await tab.evaluate(() =>
        (window as unknown as { __upd: Upd }).__upd.emit({
          event: 'update-downloaded',
          data: { version: '1.93.0' },
        })
      );
      await tab.waitForSelector('button[aria-label="Restart to update"]');
      // One element, one position: the label changed and focus rode along.
      stillTheSameFocusedControl(await probe(tab), 'Restart to update');
    } finally {
      await tab.close();
    }
  });

  it('the instrument: the old disabled attribute drops focus to the page', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(old);
    try {
      const control = tab.locator('button[aria-label="Check for updates"]');
      await control.waitFor();
      await control.focus();
      await tab.keyboard.press('Enter');
      await tab.waitForFunction(() => (window as unknown as { __upd: Upd }).__upd.calls.length > 0);
      await tab.waitForTimeout(120);
      const after = await probe(tab);
      expect(after.tag, 'the old markup must lose focus, or this case cannot fail').toBe('BODY');
    } finally {
      await tab.close();
    }
  });
});
