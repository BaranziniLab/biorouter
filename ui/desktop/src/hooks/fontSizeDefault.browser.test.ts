// @vitest-environment node
/**
 * The Text size preference measured in Chromium (owner request 2026-11-19: Small, Standard and
 * Large, with Standard, the middle one, the default).
 *
 * 1.94.5 offered Standard (1, the default), Large (1.07) and Larger (1.15). The rename keeps the
 * three sizes and moves the default one step up, so a fresh install now reads at the size 1.94.5
 * called Large, and anyone who chose a size keeps it. jsdom lays nothing out and runs no CSS, so
 * `useFontSize.test.tsx` can only say which scale is set; this file loads the shipped
 * `index.html` (its pre-hydration script) at an http origin, with `styles/main.css` compiled by
 * Tailwind and the app's font-scale pass in place of the `<link>`, and measures the body text and
 * a sidebar row:
 *
 * - before the bundle loads, a fresh install paints at the old Large size, not the old Standard;
 * - a size saved by 1.94.5 paints at the size it had there;
 * - after the renderer's startup migration and React's mount (the real `ThemeProvider` and the
 *   real Settings control), nothing moves, the middle segment is checked for a fresh install, and
 *   the old key has been moved to the new one.
 *
 * It needs a Chromium; with none installed it warns and passes, like the other `.browser.test`
 * files.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const DESKTOP = resolve(here, '../..');
const STYLES = resolve(here, '../styles');
const requireFromDesktop = createRequire(resolve(DESKTOP, 'package.json'));

/** What 1.94.5 set `--app-font-scale` to for each of its sizes. */
const BEFORE_THE_RENAME = { standard: 1, large: 1.07, larger: 1.15 } as const;

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

/**
 * What `renderer.tsx` does with the preference, in its order: the migration before React, then
 * the app's real `ThemeProvider` (which applies the size) around the real Settings control. The
 * probe row wears the sidebar's row height token.
 */
const MOUNT_ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ThemeProvider } from './contexts/ThemeContext';
import { migrateLegacyFontSize } from './hooks/useFontSize';
import FontSizeSelector from './components/settings/app/FontSizeSelector';
migrateLegacyFontSize();
createRoot(document.getElementById('root')!).render(
  <ThemeProvider>
    <FontSizeSelector />
  </ThemeProvider>
);
`;

const PROBE_CLASS = 'h-row-nav';

const noCss: Plugin = {
  name: 'nocss',
  setup(b) {
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

type Measured = { scale: string; dataset: string | undefined; body: number; row: number };

/** Chromium lays boxes out in 1/64 px units, so a scaled row height lands within one of them. */
const LAYOUT_UNIT = 1 / 64;
const expectRow = (actual: number, expected: number) =>
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(LAYOUT_UNIT);

describe('Text size in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';
  let mountScript = '';
  let indexHtml = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Text size browser tests.');
      return;
    }
    const mount = await build({
      stdin: {
        contents: MOUNT_ENTRY,
        loader: 'tsx',
        resolveDir: resolve(here, '..'),
        sourcefile: 'mount.tsx',
      },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"' },
      plugins: [noCss],
      logLevel: 'silent',
    });
    mountScript = mount.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    const tailwind = compiler.build(
      new Scanner({}).scanFiles([
        { content: mountScript, extension: 'js' },
        { content: `<div class="${PROBE_CLASS}"></div>`, extension: 'html' },
      ])
    );
    css = (await postcss([fontScale()]).process(tailwind, { from: undefined })).css;

    // The shipped page, with the stylesheet inlined where the <link> was and the React entry
    // dropped (each case adds the mount itself). Each swap is asserted, so a markup change cannot
    // leave this testing a stub.
    const source = readFileSync(resolve(DESKTOP, 'index.html'), 'utf8');
    const link = /<link href="\.\/src\/styles\/main\.css" rel="stylesheet"\/>/;
    const entry = /<script type="module" src="\.\/src\/renderer\.tsx"><\/script>/;
    if (!link.test(source) || !entry.test(source)) {
      throw new Error('index.html no longer has the stylesheet link or the renderer entry');
    }
    indexHtml = source
      .replace(link, () => `<style>${css}</style>`)
      .replace(entry, () => `<div class="${PROBE_CLASS}" data-probe="row"></div>`);
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** The real page at an http origin, with `storage` saved before any script runs. */
  async function open(storage: Record<string, string> = {}): Promise<Page> {
    const ctx = await browser!.newContext({ viewport: { width: 1200, height: 800 } });
    const tab = await ctx.newPage();
    await tab.addInitScript((entries) => {
      for (const [key, value] of Object.entries(entries)) localStorage.setItem(key, value);
    }, storage);
    await tab.route('http://text-size.test/', (route) =>
      route.fulfill({ body: indexHtml, contentType: 'text/html' })
    );
    await tab.goto('http://text-size.test/');
    return tab;
  }

  /** The same stylesheet at a fixed scale, with no script: how a size painted in 1.94.5. */
  async function reference(scale: number): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1200, height: 800 } });
    await tab.setContent(
      `<!doctype html><html style="--app-font-scale: ${scale}"><head><style>${css}</style></head>` +
        `<body><div class="${PROBE_CLASS}" data-probe="row"></div></body></html>`
    );
    return tab;
  }

  const measure = (tab: Page): Promise<Measured> =>
    tab.evaluate(() => ({
      scale: document.documentElement.style.getPropertyValue('--app-font-scale'),
      dataset: document.documentElement.dataset.fontSize,
      body: parseFloat(getComputedStyle(document.body).fontSize),
      row: document.querySelector('[data-probe="row"]')!.getBoundingClientRect().height,
    }));

  async function hydrate(tab: Page) {
    await tab.addScriptTag({ content: mountScript });
    await tab.waitForSelector('[role="radio"]');
    return tab.evaluate(() => ({
      checked: [...document.querySelectorAll('[role="radio"]')]
        .filter((radio) => radio.getAttribute('aria-checked') === 'true')
        .map((radio) => radio.textContent),
      labels: [...document.querySelectorAll('[role="radio"]')].map((radio) => radio.textContent),
      current: localStorage.getItem('biorouter:text-size'),
      legacy: localStorage.getItem('app_font_size'),
    }));
  }

  it('paints a fresh install at the size 1.94.5 called Large, before and after React mounts', async () => {
    if (!browser) return;
    const fresh = await open();
    const oldLarge = await reference(BEFORE_THE_RENAME.large);
    const oldStandard = await reference(BEFORE_THE_RENAME.standard);
    try {
      const painted = await measure(fresh);
      const large = await measure(oldLarge);
      const standard = await measure(oldStandard);
      expect(painted).toMatchObject({ scale: '1.07', dataset: 'standard' });
      expect(painted.body).toBeCloseTo(14 * 1.07, 2);
      expectRow(painted.row, 28 * 1.07);
      expect(painted.body).toBeCloseTo(large.body, 3);
      expect(painted.row).toBeCloseTo(large.row, 3);
      expect(painted.body).toBeGreaterThan(standard.body);

      const hydrated = await hydrate(fresh);
      expect(hydrated.labels).toEqual(['Small', 'Standard', 'Large']);
      expect(hydrated.checked).toEqual(['Standard']);
      // A fresh install stays unset, so it keeps following the default.
      expect(hydrated).toMatchObject({ current: null, legacy: null });
      expect(await measure(fresh)).toEqual(painted);
    } finally {
      await fresh.context().close();
      await oldLarge.close();
      await oldStandard.close();
    }
  }, 60_000);

  it.each([
    ['standard', 'Small'],
    ['large', 'Standard'],
    ['larger', 'Large'],
  ] as const)(
    "keeps a saved 1.94.5 '%s' at its size, shown as %s",
    async (old, label) => {
      if (!browser) return;
      const tab = await open({ app_font_size: old });
      const before = await reference(BEFORE_THE_RENAME[old]);
      try {
        const painted = await measure(tab);
        const then = await measure(before);
        expect(painted.body).toBeCloseTo(then.body, 3);
        expect(painted.row).toBeCloseTo(then.row, 3);

        const hydrated = await hydrate(tab);
        expect(hydrated.checked).toEqual([label]);
        expect(hydrated).toMatchObject({ current: label.toLowerCase(), legacy: null });
        expect(await measure(tab)).toEqual(painted);
      } finally {
        await tab.context().close();
        await before.close();
      }
    },
    60_000
  );

  it.each([
    ['small', 1],
    ['standard', 1.07],
    ['large', 1.15],
  ] as const)(
    "paints a saved '%s' at %sx",
    async (size, scale) => {
      if (!browser) return;
      const tab = await open({ 'biorouter:text-size': size });
      try {
        const painted = await measure(tab);
        expect(painted).toMatchObject({ scale: String(scale), dataset: size });
        expect(painted.body).toBeCloseTo(14 * scale, 2);
        expectRow(painted.row, 28 * scale);
      } finally {
        await tab.context().close();
      }
    },
    60_000
  );
});
