// @vitest-environment node
/**
 * The day-one privacy notice is the shared dialog, measured in Chromium with the whole stylesheet
 * (first run vision round 1, FR-R1-14; R-34).
 *
 * On an upgraded install this is the very first thing on screen, and it was the one raw
 * `DialogContent` on the first-run path: 560px wide, vertically centred (it sat at y=200 in a 900px
 * window where every other dialog's top edge is at 90), no header rule, a bare "Got it" row under the
 * text. jsdom lays nothing out, so `FirstRunPrivacyNotice.test.tsx` can only say which attributes the
 * shell sets; this file bundles the real notice with esbuild (the daemon client and the proof-of-user
 * helper stubbed, the counts passed in as the notice's own `counts` prop allows), compiles
 * `styles/main.css` with Tailwind over the classes the bundle uses, runs the renderer's text-size
 * pass, and measures.
 *
 * It also measures the one thing a long notice owes a short window: the body scrolls and the
 * "Got it" row stays on screen, because the notice is a statement the person must be able to
 * acknowledge.
 *
 * To see these cases fail on another version of the component, point `FIRST_RUN_NOTICE_SOURCE` at
 * that file: it is loaded in place of `./FirstRunPrivacyNotice`, its imports resolved from this
 * directory.
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
    contents: `export const listSessions = async () => new Promise(() => {});`,
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
        args.importer.endsWith('FirstRunPrivacyNotice.tsx') || args.namespace === 'notice-source'
          ? { path: args.path, namespace: 'stub', pluginData: contents }
          : undefined
      );
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      loader: 'js',
    }));
    const source = process.env.FIRST_RUN_NOTICE_SOURCE;
    if (source) {
      b.onResolve({ filter: /^\.\/FirstRunPrivacyNotice$/ }, () => ({
        path: source,
        namespace: 'notice-source',
      }));
      b.onLoad({ filter: /.*/, namespace: 'notice-source' }, (args) => ({
        contents: readFileSync(args.path, 'utf8'),
        resolveDir: here,
        loader: 'tsx',
      }));
    }
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

/**
 * The numbers of an upgraded install, as the notice quotes them: the longest realistic content (a
 * four-row list, the unknown-model line, and the clinical-extension paragraph), so the height
 * measured is the worst the dialog is asked to hold.
 */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { FirstRunPrivacyNotice } from './FirstRunPrivacyNotice';

const counts = {
  privateVisible: 791,
  backfilledVisible: 791,
  publicNamedVisible: 1288,
  unknownProviderVisible: 3463,
  totalVisible: 5542,
  privateByProvider: [
    { provider: 'versa_azure', count: 505 },
    { provider: 'ollama', count: 137 },
    { provider: 'versa_bedrock', count: 119 },
    { provider: 'llamacpp', count: 30 },
  ],
};

createRoot(document.getElementById('root')!).render(
  <FirstRunPrivacyNotice open onDismiss={() => {}} counts={counts} publicClinicalExtensions={['medcp']} />
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
  body: { scrollHeight: number; clientHeight: number; overflowY: string };
  footerRule: number;
  button: Box;
  prose: { fontSize: string; lineHeight: string; testId: string }[];
};

describe('The privacy notice is the shared dialog (FR-R1-14)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the privacy notice dialog tests.');
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

  async function open(
    options: { width?: number; height?: number; dark?: boolean } = {}
  ): Promise<Page> {
    const { width = 1440, height = 900, dark = false } = options;
    const tab = await browser!.newPage({ viewport: { width, height } });
    await tab.setContent(
      `<!doctype html><html${dark ? ' class="dark"' : ''}><head><style>${css}</style></head>` +
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
      const panel = document.querySelector('[role="dialog"]') as HTMLElement;
      const heading = panel.querySelector('h2')!;
      // The header is the band the title sits in: the panel's first child.
      const header = panel.firstElementChild!;
      // The scroller is the nearest scrolling ancestor of the text, or the panel when none scrolls
      // (the old raw dialog), so a version without one fails on the assertions, not on a throw.
      let scroller = panel;
      for (
        let el = panel.querySelector('[data-testid="notice-headline"]')!.parentElement;
        el && el !== panel;
        el = el.parentElement
      ) {
        if (/auto|scroll/.test(getComputedStyle(el).overflowY)) {
          scroller = el;
          break;
        }
      }
      const ack = panel.querySelector('[data-testid="notice-acknowledge"]') as HTMLElement;
      const footer = ack.parentElement as HTMLElement;
      const prose = [
        'notice-headline',
        'notice-last-model-caveat',
        'notice-knowledge-bases',
        'notice-public-clinical-extensions',
      ].map((testId) => {
        const style = getComputedStyle(panel.querySelector(`[data-testid="${testId}"]`)!);
        return { testId, fontSize: style.fontSize, lineHeight: style.lineHeight };
      });
      return {
        panel: box(panel),
        border: panel.clientLeft,
        heading: box(heading),
        headerRule: parseFloat(getComputedStyle(header).borderBottomWidth),
        headerFills: [...header.querySelectorAll('*')]
          .map((el) => getComputedStyle(el).backgroundColor)
          .filter((color) => color !== 'rgba(0, 0, 0, 0)' && color !== 'transparent'),
        body: {
          scrollHeight: scroller.scrollHeight,
          clientHeight: scroller.clientHeight,
          overflowY: getComputedStyle(scroller).overflowY,
        },
        footerRule: parseFloat(getComputedStyle(footer).borderTopWidth),
        button: box(ack),
        prose,
      };
    });

  it('is pinned at the top and 480px wide, not centred at 560px', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab);
      // ModalShell's top anchor: max(10vh, 48px) = 90px in a 900px window.
      expect(m.panel.y).toBe(90);
      expect(m.panel.width).toBe(480);
      expect(m.panel.x).toBe((1440 - 480) / 2);
    } finally {
      await tab.close();
    }
  });

  it('draws the header rule, and the title starts at the gutter', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab);
      expect(m.headerRule).toBe(1);
      expect(m.heading.x - m.panel.x - m.border).toBe(16);
      expect(m.headerFills).toEqual([]);
    } finally {
      await tab.close();
    }
  });

  it('sets every paragraph at the body role, 14px on a 20px line', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab);
      expect(m.prose.map((p) => p.testId)).toEqual([
        'notice-headline',
        'notice-last-model-caveat',
        'notice-knowledge-bases',
        'notice-public-clinical-extensions',
      ]);
      for (const { testId, fontSize, lineHeight } of m.prose) {
        expect(fontSize, testId).toBe('14px');
        expect(lineHeight, testId).toBe('20px');
      }
    } finally {
      await tab.close();
    }
  });

  it('holds its whole text in a 900px window, with the 32px "Got it" at the footer gutter', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab);
      // Nothing is clipped or scrolled: the dialog is as tall as the text it carries.
      expect(m.body.scrollHeight).toBeLessThanOrEqual(m.body.clientHeight);
      expect(m.panel.bottom).toBeLessThanOrEqual(900 - 16);
      expect(m.footerRule).toBe(1);
      expect(m.button.height).toBe(32);
      expect(Math.abs(m.panel.right - m.border - m.button.right - 16)).toBeLessThan(0.5);
      expect(Math.abs(m.panel.bottom - m.border - m.button.bottom - 16)).toBeLessThan(0.5);
    } finally {
      await tab.close();
    }
  });

  it('scrolls its body in a short window and keeps "Got it" on screen', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ width: 1048, height: 560 });
    try {
      const m = await measure(tab);
      expect(m.body.overflowY).toBe('auto');
      expect(m.body.scrollHeight).toBeGreaterThan(m.body.clientHeight);
      // The shell's max-height keeps the bottom edge inside the window, and the button with it.
      expect(m.panel.bottom).toBeLessThanOrEqual(560 - 16);
      expect(m.button.bottom).toBeLessThanOrEqual(560 - 16);
      expect(m.button.y).toBeGreaterThan(m.panel.y);
    } finally {
      await tab.close();
    }
  });

  it('keeps the same geometry in dark', async (ctx) => {
    if (!browser) return ctx.skip();
    const light = await open();
    const dark = await open({ dark: true });
    try {
      const a = await measure(light);
      const b = await measure(dark);
      expect(b.panel).toEqual(a.panel);
      expect(b.heading).toEqual(a.heading);
      expect(b.button).toEqual(a.button);
    } finally {
      await light.close();
      await dark.close();
    }
  });
});
