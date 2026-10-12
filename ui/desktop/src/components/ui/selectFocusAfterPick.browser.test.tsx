// @vitest-environment node
/**
 * A Select pick keeps a visible keyboard focus, in Chromium, inside the real dialog (vision round 8,
 * V3-R8-2; checklist 10).
 *
 * In New schedule the cron picker's "AM or PM" Select (portalled, not searchable) took ArrowDown,
 * ArrowUp, Enter, picked AM, and then `document.activeElement` was the `role="dialog"` box: the
 * wrapper forced `blurInputOnSelect` on, and Radix's focus trap moved the focus that fell to
 * `<body>` onto the dialog surface, which draws no ring by design. The next Tab started over at
 * Name. jsdom has neither a real focus trap nor the stylesheet, so this bundles the real
 * `ModalShell` and `Select` with esbuild, compiles `styles/main.css` with Tailwind and reads what
 * is focused and whether its edge is painted. The same form with `blurInputOnSelect` passed is the
 * instrument: it must reproduce the finding, or the other cases could pass vacuously.
 *
 * To run these cases against another version of the Select, point `SELECT_SOURCE` at that file:
 * it is loaded in place of `./Select`, its imports resolved from this directory.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const sourcePlugin: Plugin = {
  name: 'select-source',
  setup(b) {
    const source = process.env.SELECT_SOURCE;
    if (source) {
      b.onResolve({ filter: /^\.\/Select$/ }, () => ({ path: source, namespace: 'select-source' }));
      b.onLoad({ filter: /.*/, namespace: 'select-source' }, (args) => ({
        contents: readFileSync(args.path, 'utf8'),
        resolveDir: here,
        loader: 'tsx',
      }));
    }
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

/** `window.__blur` is the instrument switch: pass `blurInputOnSelect` as the old default did. */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ModalShell } from '../ModalShell';
import { Button } from './button';
import { Select } from './Select';

const PERIODS = [
  { value: 'AM', label: 'AM' },
  { value: 'PM', label: 'PM' },
];
const blur = (window as unknown as { __blur: boolean }).__blur;

function App() {
  const [period, setPeriod] = React.useState('PM');
  return (
    <ModalShell
      open
      onOpenChange={() => {}}
      purpose="form"
      title="New schedule"
      footer={
        <>
          <Button variant="secondary">Cancel</Button>
          <Button>Create</Button>
        </>
      }
    >
      <div style={{ display: 'grid', gap: 12, paddingBottom: 12 }}>
        <input aria-label="Name" className="br-field" />
        <div style={{ width: 120 }}>
          <Select
            classNamePrefix="period"
            aria-label="AM or PM"
            isSearchable={false}
            options={PERIODS}
            menuPlacement="auto"
            menuPortalTarget={document.body}
            value={PERIODS.find((option) => option.value === period)}
            onChange={(next: unknown) => setPeriod((next as { value: string }).value)}
            {...(blur ? { blurInputOnSelect: true } : {})}
          />
        </div>
        <output id="period">{period}</output>
      </div>
    </ModalShell>
  );
}
createRoot(document.getElementById('root')!).render(<App />);
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

type Focus = {
  /** What holds focus: the Select's input, the dialog box, a button by its name, or something else. */
  active: string;
  focusVisible: boolean;
  /** The Select trigger's painted edge, and the focus edge token it should be. */
  edge: string;
  focusEdge: string;
  picked: string;
  menuOpen: boolean;
};

describe('a Select pick keeps a visible focus on the Select (V3-R8-2)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Select focus-after-pick tests.');
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
      loader: { '.svg': 'dataurl', '.png': 'dataurl' },
      plugins: [sourcePlugin],
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(blur: boolean): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1440, height: 900 } });
    await tab.setContent(
      `<!doctype html><html class="light"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div>` +
        `<script>window.__blur = ${blur};</script></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[role="dialog"] .period__control');
    // The open motion settles before anything is read.
    await tab.waitForTimeout(400);
    return tab;
  }

  const focus = (tab: Page): Promise<Focus> =>
    tab.evaluate(() => {
      const el = document.activeElement as HTMLElement | null;
      const input = document.querySelector('.period__control input');
      const dialog = document.querySelector('[role="dialog"]');
      const active =
        el === input
          ? 'select'
          : el === dialog
            ? 'dialog'
            : el?.tagName === 'BUTTON'
              ? `button:${el.textContent?.trim()}`
              : (el?.getAttribute('aria-label') ?? el?.tagName ?? 'none');
      const control = document.querySelector('.period__control')!;
      const probe = document.createElement('span');
      probe.style.color = 'var(--border-focus)';
      document.body.append(probe);
      // Chromium writes an opaque colour as rgb() or as rgba(..., 1) depending on how it was
      // reached (a transition's end value is rgba), so both sides are read as four numbers.
      const rgba = (value: string) => {
        const [r, g, b, a = 1] = (value.match(/[\d.]+/g) ?? []).map(Number);
        return `${r},${g},${b},${a}`;
      };
      const focusEdge = rgba(getComputedStyle(probe).color);
      probe.remove();
      return {
        active,
        focusVisible: el?.matches(':focus-visible') ?? false,
        edge: rgba(getComputedStyle(control).borderTopColor),
        focusEdge,
        picked: document.getElementById('period')?.textContent ?? '',
        menuOpen: Boolean(document.querySelector('.period__menu')),
      };
    });

  /** The finding's keys: Tab from Name to the Select, then ArrowDown, ArrowUp, Enter. */
  async function pickByKeyboard(tab: Page) {
    await tab.click('[aria-label="Name"]');
    await tab.keyboard.press('Tab');
    await tab.keyboard.press('ArrowDown');
    await tab.waitForSelector('.period__menu');
    await tab.keyboard.press('ArrowUp');
    await tab.keyboard.press('Enter');
    await tab.waitForSelector('.period__menu', { state: 'detached' });
    // Past the trigger's border transition (--dur-fast).
    await tab.waitForTimeout(300);
  }

  it('the blurring Select loses focus to the dialog box, and Tab restarts at Name (instrument)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(true);
    try {
      await pickByKeyboard(tab);
      const after = await focus(tab);
      expect(after.picked).toBe('AM');
      expect(after.active).toBe('dialog');
      expect(after.edge).not.toBe(after.focusEdge);
      await tab.keyboard.press('Tab');
      expect((await focus(tab)).active).toBe('Name');
    } finally {
      await tab.close();
    }
  });

  it('keyboard: the pick keeps focus on the Select with its edge painted, and Tab reaches Cancel', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(false);
    try {
      await pickByKeyboard(tab);
      const after = await focus(tab);
      expect(after).toMatchObject({ picked: 'AM', menuOpen: false, active: 'select' });
      expect(after.focusVisible).toBe(true);
      expect(after.focusEdge).not.toBe('0,0,0,0');
      expect(after.edge).toBe(after.focusEdge);
      await tab.keyboard.press('Tab');
      expect((await focus(tab)).active).toBe('button:Cancel');
    } finally {
      await tab.close();
    }
  });

  it('pointer: a clicked option leaves focus on the Select, not on the dialog box', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(false);
    try {
      await tab.click('.period__control');
      await tab.waitForSelector('.period__menu');
      await tab.click('.period__option >> text=AM');
      await tab.waitForSelector('.period__menu', { state: 'detached' });
      await tab.waitForTimeout(300);
      const after = await focus(tab);
      expect(after).toMatchObject({ picked: 'AM', active: 'select' });
      expect(after.edge).toBe(after.focusEdge);
    } finally {
      await tab.close();
    }
  });
});
