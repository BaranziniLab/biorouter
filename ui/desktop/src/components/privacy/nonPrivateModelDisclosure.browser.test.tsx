// @vitest-environment node
/**
 * The non-private model disclosure is the shared dialog, measured in Chromium with the whole
 * stylesheet (ruling 23; spec 2.6, V3-R7-1; R-34).
 *
 * It is on the first run path of anyone who finishes setup with a public provider and sends a
 * first message, and it was the last raw `DialogContent` there: 560px wide, vertically centred, no
 * header rule, its prose scrolling in a `max-h-[50vh]` box of its own. It now renders through
 * `ModalShell` lg. jsdom lays nothing out, so `NonPrivateModelDisclosure.required.test.tsx` pins the
 * dismissal rule and the focus target there; this file bundles the real disclosure with esbuild
 * (the daemon client and the proof-of-user helper stubbed: the dialog takes its copy as a prop),
 * compiles `styles/main.css` with Tailwind over the classes the bundle uses, runs the renderer's
 * text-size pass, and measures.
 *
 * It also measures what the swap could have broken for a person reading it: focus rests on the
 * prose with no ring, the keyboard still scrolls the text when the window is short (the scroller
 * is now the shell's body, not the focused prose), and a refused acknowledgement stays in view
 * beside the button that changed its word, at the app's minimum window and the largest text size.
 *
 * The copy is a FIXTURE of the served copy's length and shape (three paragraphs, two emphasised
 * clauses), never the product's words: the one definition lives in
 * `crates/biorouter/src/privacy/disclosure.rs`, and a second copy here would be counted by the
 * single-definition gate.
 *
 * To see these cases fail on another version of the component, point `DISCLOSURE_SOURCE` at that
 * file: it is loaded in place of `./NonPrivateModelDisclosure`, its imports resolved from this
 * directory. Set `DISCLOSURE_SHOTS_DIR` to also save screenshots there.
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
const SHOTS = process.env.DISCLOSURE_SHOTS_DIR;

const STUBS: { filter: RegExp; contents: string }[] = [
  {
    filter: /^\.\.\/\.\.\/api$/,
    contents:
      `export const getPrivacyDisclosure = async () => new Promise(() => {});\n` +
      `export const ackPrivacyDisclosure = async () => new Promise(() => {});`,
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
        args.importer.endsWith('disclosureCopy.ts')
          ? { path: args.path, namespace: 'stub', pluginData: contents }
          : undefined
      );
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      loader: 'js',
    }));
    const source = process.env.DISCLOSURE_SOURCE;
    if (source) {
      b.onResolve({ filter: /^\.\/NonPrivateModelDisclosure$/ }, () => ({
        path: source,
        namespace: 'disclosure-source',
      }));
      b.onLoad({ filter: /.*/, namespace: 'disclosure-source' }, (args) => ({
        contents: readFileSync(args.path, 'utf8'),
        resolveDir: here,
        loader: 'tsx',
      }));
    }
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

/** About the served copy's length (1.5k characters), in its shape: three paragraphs, two marked. */
const LONG = [
  'FIXTURE-PARAGRAPH-ONE. This model runs on servers your institution does not operate. ' +
    'What you type, the files you attach and the results of every tool it calls leave this ' +
    'computer for that provider, under its terms rather than your institution’s agreements, and ' +
    'the provider decides how long they are kept and who may read them.',
  'FIXTURE-PARAGRAPH-TWO. Biorouter keeps three things away from it: chats marked private, ' +
    'knowledge bases marked private and the credentials it stores for you. Those stay with the ' +
    'models your institution hosts, and a chat that has touched them cannot be sent here, ' +
    'whichever model you pick later in the same chat.',
  'FIXTURE-PARAGRAPH-THREE. It **does not stop this model from reading ordinary files on this ' +
    'computer** when a tool asks for them, and it **does not make anything you type here ' +
    'private**. Treat the conversation as shared with the provider, keep patient data and ' +
    'unpublished results out of it, and switch to a model your institution hosts before you ' +
    'work with either one, so the rule above can do its job.',
].join('\n\n');

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { NonPrivateModelDisclosure } from './NonPrivateModelDisclosure';

const copy = {
  titleTemplate: '{provider} is not hosted by your institution.',
  long: ${JSON.stringify(LONG)},
  short: 'FIXTURE-SHORT',
};

createRoot(document.getElementById('root')!).render(
  <NonPrivateModelDisclosure
    open
    providerDisplayName="OpenAI"
    copy={copy}
    onAcknowledge={() => {}}
    acknowledgeError={window.__DISCLOSURE_ERROR ?? null}
  />
);
`;

const ACK_ERROR =
  'Your acknowledgement was not saved, because this app could not prove that a person answered. ' +
  'You will see this once more next time.';

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
  footerRule: number;
  scroller: {
    scrollHeight: number;
    clientHeight: number;
    scrollTop: number;
    overflowY: string;
    bottom: number;
  };
  /** Whether the prose, the node focus rests on, is itself the scroller (the old raw dialog). */
  proseScrolls: boolean;
  button: Box;
  buttonText: string;
  error: Box | null;
  closeButtons: number;
  focus: { onProse: boolean; outline: string; boxShadow: string };
  maxWeight: number;
};

describe('The non-private model disclosure is the shared dialog (ruling 23)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the non-private model disclosure dialog tests.');
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
    options: {
      width?: number;
      height?: number;
      dark?: boolean;
      scale?: number;
      error?: boolean;
      /** The screenshot's name, when `DISCLOSURE_SHOTS_DIR` is set. */
      shot?: string;
    } = {}
  ): Promise<Page> {
    const { width = 1440, height = 900, dark = false, scale = 1, error = false, shot } = options;
    const tab = await browser!.newPage({ viewport: { width, height } });
    await tab.setContent(
      `<!doctype html><html${dark ? ' class="dark"' : ''} style="--app-font-scale:${scale}">` +
        `<head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div>` +
        `<script>window.__DISCLOSURE_ERROR = ${error ? JSON.stringify(ACK_ERROR) : 'null'};</script>` +
        `</body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[role="dialog"]');
    // The open motion (250ms) settles before anything is read.
    await tab.waitForTimeout(400);
    if (SHOTS && shot) await tab.screenshot({ path: resolve(SHOTS, `${shot}.png`) });
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
      const header = panel.firstElementChild!;
      const prose = [...panel.querySelectorAll('p')].find((p) =>
        p.textContent?.startsWith('FIXTURE-PARAGRAPH-ONE')
      )!.parentElement as HTMLElement;
      // The nearest scrolling box at or above the prose, or the panel when none scrolls.
      let scroller: HTMLElement = panel;
      for (let el: HTMLElement | null = prose; el && el !== panel; el = el.parentElement) {
        if (/auto|scroll/.test(getComputedStyle(el).overflowY)) {
          scroller = el;
          break;
        }
      }
      const button = [...panel.querySelectorAll('button')].find((b) =>
        /I understand|Continue/.test(b.textContent ?? '')
      )!;
      const error = panel.querySelector('[data-testid="disclosure-ack-error"]');
      const focused = document.activeElement as HTMLElement;
      const focusStyle = getComputedStyle(focused);
      return {
        panel: box(panel),
        border: panel.clientLeft,
        heading: box(heading),
        headerRule: parseFloat(getComputedStyle(header).borderBottomWidth),
        footerRule: parseFloat(getComputedStyle(button.parentElement!).borderTopWidth),
        scroller: {
          scrollHeight: scroller.scrollHeight,
          clientHeight: scroller.clientHeight,
          scrollTop: scroller.scrollTop,
          bottom: scroller.getBoundingClientRect().bottom,
          overflowY: getComputedStyle(scroller).overflowY,
        },
        proseScrolls: scroller === prose,
        button: box(button),
        buttonText: button.textContent ?? '',
        error: error ? box(error) : null,
        closeButtons: [...panel.querySelectorAll('button')].filter(
          (b) => b.textContent?.trim() === 'Close'
        ).length,
        focus: {
          onProse: focused === prose,
          outline: focusStyle.outlineStyle,
          boxShadow: focusStyle.boxShadow,
        },
        maxWeight: Math.max(
          ...[...panel.querySelectorAll('*')].map((el) =>
            parseInt(getComputedStyle(el).fontWeight, 10)
          )
        ),
      };
    });

  it('is pinned at the top and 640px wide, not centred at 560px', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ shot: 'disclosure-light-1440' });
    try {
      const m = await measure(tab);
      // ModalShell's top anchor: max(10vh, 48px) = 90px in a 900px window.
      expect(m.panel.y).toBe(90);
      expect(m.panel.width).toBe(640);
      expect(m.panel.x).toBe((1440 - 640) / 2);
    } finally {
      await tab.close();
    }
  });

  it('draws the header rule, the title at the gutter, and no ×', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab);
      expect(m.headerRule).toBe(1);
      expect(m.heading.x - m.panel.x - m.border).toBe(16);
      expect(m.closeButtons).toBe(0);
      expect(m.buttonText).toBe('I understand');
      expect(m.button.height).toBe(32);
      expect(m.footerRule).toBe(1);
      expect(Math.abs(m.panel.right - m.border - m.button.right - 16)).toBeLessThan(0.5);
      expect(Math.abs(m.panel.bottom - m.border - m.button.bottom - 16)).toBeLessThan(0.5);
      // The emphasised clauses are 600 (V4-R7-6), and nothing is heavier.
      expect(m.maxWeight).toBeLessThanOrEqual(600);
    } finally {
      await tab.close();
    }
  });

  it('opens with focus on the prose, drawn without a ring, and Escape leaves it there', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab);
      expect(m.focus.onProse).toBe(true);
      expect(m.focus.outline).toBe('none');
      expect(m.focus.boxShadow).toBe('none');
      await tab.keyboard.press('Escape');
      await tab.waitForTimeout(300);
      expect((await measure(tab)).focus.onProse).toBe(true);
      // A press on the backdrop: the dialog is still the one thing to answer. (Whether the
      // dialog stays open is the gate's `open`; that nothing asks to close it is pinned in jsdom.)
      await tab.mouse.click(8, 8);
      await tab.waitForTimeout(300);
      expect(await tab.locator('[role="dialog"]').count()).toBe(1);
    } finally {
      await tab.close();
    }
  });

  it('holds the whole text in a 900px window', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab);
      expect(m.scroller.scrollHeight).toBeLessThanOrEqual(m.scroller.clientHeight);
      expect(m.panel.bottom).toBeLessThanOrEqual(900 - 16);
    } finally {
      await tab.close();
    }
  });

  // The app's minimum window (main.ts: 1048 x 600) at the largest text size (`larger`, 1.15): the
  // smallest room a person can give the dialog. Text of the served copy's length overflows there
  // (measured: 400px of text in a 377px body), and a refusal's notice takes more room still.
  const SHORT = { width: 1048, height: 600, scale: 1.15 } as const;

  it('scrolls by keyboard from the prose in a short window, the button staying on screen', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(SHORT);
    try {
      const before = await measure(tab);
      expect(before.scroller.overflowY).toBe('auto');
      expect(before.scroller.scrollHeight).toBeGreaterThan(before.scroller.clientHeight);
      expect(before.proseScrolls).toBe(false);
      expect(before.focus.onProse).toBe(true);
      expect(before.panel.bottom).toBeLessThanOrEqual(600 - 16);
      expect(before.button.bottom).toBeLessThanOrEqual(600 - 16);

      await tab.keyboard.press('PageDown');
      await tab.waitForTimeout(400);
      const after = await measure(tab);
      expect(after.scroller.scrollTop).toBeGreaterThan(0);
      expect(after.focus.onProse).toBe(true);
    } finally {
      await tab.close();
    }
  });

  it('keeps a refused acknowledgement in view above "Continue" in that short window', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ ...SHORT, error: true, shot: 'disclosure-refused-1048x600-larger' });
    try {
      const m = await measure(tab);
      expect(m.buttonText).toBe('Continue');
      // The text still scrolls; the notice is not part of what scrolls: it sits below the
      // scroller's bottom edge and above the button, wholly inside the window.
      expect(m.scroller.scrollHeight).toBeGreaterThan(m.scroller.clientHeight);
      expect(m.error).not.toBeNull();
      const error = m.error!;
      expect(error.y).toBeGreaterThanOrEqual(m.scroller.bottom);
      expect(error.bottom).toBeLessThanOrEqual(m.button.y);
      expect(m.button.bottom).toBeLessThanOrEqual(600 - 16);
      // The footer's full width: the button's right edge, and the same gutter on both sides
      // (16px scaled with the text, as every footer's is).
      expect(Math.abs(error.right - m.button.right)).toBeLessThan(0.5);
      const left = error.x - m.panel.x - m.border;
      const right = m.panel.right - m.border - error.right;
      expect(Math.abs(left - right)).toBeLessThan(0.5);
      expect(Math.abs(left - 16 * SHORT.scale)).toBeLessThan(0.5);
    } finally {
      await tab.close();
    }
  });

  it('keeps the same geometry in dark', async (ctx) => {
    if (!browser) return ctx.skip();
    const light = await open();
    const dark = await open({ dark: true, shot: 'disclosure-dark-1440' });
    try {
      const a = await measure(light);
      const b = await measure(dark);
      expect(b.panel).toEqual(a.panel);
      expect(b.heading).toEqual(a.heading);
      expect(b.button).toEqual(a.button);
      expect(b.focus.onProse).toBe(true);
    } finally {
      await light.close();
      await dark.close();
    }
  });
});
