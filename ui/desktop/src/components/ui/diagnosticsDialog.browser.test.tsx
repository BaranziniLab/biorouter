// @vitest-environment node
/**
 * "Report a problem" is the shared dialog, measured in Chromium with the whole stylesheet (vision
 * V4-R4-2, coordinator ruling 1).
 *
 * It was the one dialog left on a raw `DialogContent`: centred on the window (it sat at y=251 in a
 * 900px window where every other dialog's top edge is at 90), no header rule, a 36px warning tile
 * before the title, and 28px footer buttons against 32px in every ModalShell footer. jsdom lays
 * nothing out, so `Diagnostics.test.tsx` can only say which attributes the shell sets; this file
 * bundles the real `DiagnosticsModal` with esbuild (the daemon client, toasts and the proof-of-user
 * helper stubbed: the daemon answers that the chat is private), compiles `styles/main.css` with
 * Tailwind over the classes the bundle uses, runs the renderer's text-size pass, and measures.
 *
 * The private-chat warning arrives after the dialog opens (the modal asks the daemon for the tier),
 * so the dialog grows while it is open: the top edge must not move when it does.
 *
 * To see these cases fail on another version of the component, point `DIAGNOSTICS_SOURCE` at that
 * file: it is loaded in place of `./Diagnostics`, its imports resolved from this directory.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const STUBS: { filter: RegExp; contents: string }[] = [
  {
    filter: /^\.\.\/\.\.\/api$/,
    contents: `
      export const diagnostics = async () => new Promise(() => {});
      export const systemInfo = async () => new Promise(() => {});
      // The tier arrives after the dialog opens, as it does from the daemon: the test says when.
      const tier = new Promise((release) => { globalThis.__releaseTier = release; });
      export const getSession = () => tier.then(() => ({ data: { privacy_tier: 'private' } }));`,
  },
  {
    filter: /^\.\.\/\.\.\/toasts$/,
    contents: `export const toastError = () => {}; export const toastSuccess = () => {};`,
  },
  {
    filter: /^\.\.\/\.\.\/utils\/userAction$/,
    contents: `export const userActionHeaders = async () => ({});`,
  },
];

const stubPlugin: Plugin = {
  name: 'stubs',
  setup(b) {
    for (const { filter, contents } of STUBS) {
      b.onResolve({ filter }, (args) =>
        args.importer.endsWith('Diagnostics.tsx') || args.namespace === 'diagnostics-source'
          ? { path: args.path, namespace: 'stub', pluginData: contents }
          : undefined
      );
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      loader: 'js',
    }));
    const source = process.env.DIAGNOSTICS_SOURCE;
    if (source) {
      b.onResolve({ filter: /^\.\/Diagnostics$/ }, () => ({
        path: source,
        namespace: 'diagnostics-source',
      }));
      b.onLoad({ filter: /.*/, namespace: 'diagnostics-source' }, (args) => ({
        contents: readFileSync(args.path, 'utf8'),
        resolveDir: here,
        loader: 'tsx',
      }));
    }
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { DiagnosticsModal } from './Diagnostics';

createRoot(document.getElementById('root')!).render(
  <DiagnosticsModal isOpen onClose={() => {}} sessionId="20261011_1" />
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

type Box = { x: number; y: number; width: number; height: number; right: number; bottom: number };

type Measure = {
  panel: Box;
  /** The panel's border, so a gutter is read from inside it. */
  border: number;
  heading: Box;
  headerRule: number;
  headerFills: string[];
  buttons: { name: string; box: Box }[];
  warning: boolean;
};

describe('Report a problem is the shared dialog (V4-R4-2)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Report a problem dialog tests.');
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
      plugins: [stubPlugin],
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    const tailwind = compiler.build(
      new Scanner({}).scanFiles([{ content: script, extension: 'js' }])
    );
    css = (await postcss([fontScale()]).process(tailwind, { from: undefined })).css;
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(scale: '1' | '1.07' = '1'): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1440, height: 900 } });
    await tab.setContent(
      `<!doctype html><html style="--app-font-scale: ${scale}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[role="dialog"]');
    // The open motion (250ms) settles before anything is read.
    await tab.waitForTimeout(400);
    return tab;
  }

  const measure = (tab: Page): Promise<Measure> =>
    tab.evaluate(() => {
      const box = (el: Element) => {
        const r = el.getBoundingClientRect();
        return {
          x: r.left,
          y: r.top,
          width: r.width,
          height: r.height,
          right: r.right,
          bottom: r.bottom,
        };
      };
      const panel = document.querySelector('[role="dialog"]')!;
      const heading = panel.querySelector('h2')!;
      // The header is the band the title sits in: the panel's first child.
      const header = panel.firstElementChild!;
      const tip = panel.querySelector('.br-info-tip');
      const headerFills = [...header.querySelectorAll('*')]
        .filter((el) => !(tip && (el === tip || tip.contains(el))))
        .map((el) => getComputedStyle(el).backgroundColor)
        .filter((color) => color !== 'rgba(0, 0, 0, 0)' && color !== 'transparent');
      const buttons = [...panel.querySelectorAll('button')]
        .filter((b) => b.textContent && !b.closest('.br-info-tip') && b.textContent !== 'Close')
        .map((b) => ({ name: b.textContent!.trim(), box: box(b) }));
      return {
        panel: box(panel),
        border: (panel as HTMLElement).clientLeft,
        heading: box(heading),
        headerRule: parseFloat(getComputedStyle(header).borderBottomWidth),
        headerFills,
        buttons,
        warning: !!panel.querySelector('[data-testid="diagnostics-private-warning"]'),
      };
    });

  it('is pinned at the top, 480px wide, and stays put when the private warning arrives', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const before = await measure(tab);
      expect(before.warning).toBe(false);
      await tab.evaluate(() => (globalThis as { __releaseTier?: () => void }).__releaseTier?.());
      await tab.waitForSelector('[data-testid="diagnostics-private-warning"]');
      await tab.waitForTimeout(100);
      const after = await measure(tab);
      expect(after.warning).toBe(true);
      // ModalShell's top anchor: max(10vh, 48px) = 90px in a 900px window.
      expect(before.panel.y).toBe(90);
      expect(after.panel.y).toBe(90);
      expect(after.panel.height).toBeGreaterThan(before.panel.height);
      expect(after.panel.width).toBe(480);
      expect(after.panel.x).toBe((1440 - 480) / 2);
    } finally {
      await tab.close();
    }
  });

  it('draws the header rule, and the title starts at the gutter with no tile before it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab);
      expect(m.headerRule).toBe(1);
      expect(m.heading.x - m.panel.x - m.border).toBe(16);
      expect(m.headerFills).toEqual([]);
      // The InfoTip sits beside the title, on its line.
      const tip = await tab.evaluate(() => {
        const glyph = document.querySelector('[role="dialog"] .br-info-tip svg')!;
        const r = glyph.getBoundingClientRect();
        return { x: r.left, mid: r.top + r.height / 2 };
      });
      expect(tip.x).toBeGreaterThanOrEqual(m.heading.right);
      expect(tip.x - m.heading.right).toBeLessThanOrEqual(6);
      expect(Math.abs(tip.mid - (m.heading.y + m.heading.height / 2))).toBeLessThanOrEqual(1);
    } finally {
      await tab.close();
    }
  });

  for (const scale of ['1', '1.07'] as const) {
    it(`has 32px footer buttons on one row at the trailing gutter (text size ${scale})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(scale);
      try {
        const m = await measure(tab);
        expect(m.buttons.map((b) => b.name)).toEqual([
          'Cancel',
          'Generate diagnostics',
          'File a bug on GitHub',
        ]);
        // The default rung: 32px, and Large text scales the control ladder with the type.
        const control = 32 * Number(scale);
        for (const { name, box } of m.buttons) {
          expect(Math.abs(box.height - control), name).toBeLessThan(0.01);
          expect(box.y, name).toBe(m.buttons[0].box.y);
        }
        // The shell's 16px gutters, which Large text scales with the type.
        const gutter = 16 * Number(scale);
        expect(Math.abs(m.panel.right - m.border - m.buttons[2].box.right - gutter)).toBeLessThan(
          0.5
        );
        expect(Math.abs(m.panel.bottom - m.border - m.buttons[2].box.bottom - gutter)).toBeLessThan(
          0.5
        );
      } finally {
        await tab.close();
      }
    });
  }
});
