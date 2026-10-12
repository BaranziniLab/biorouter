// @vitest-environment node
/**
 * The emphasised words in Settings, Privacy are 600, never 700 (vision V4-R7-6; principle 3),
 * measured in Chromium with the whole stylesheet.
 *
 * A bare `<strong>` takes Tailwind preflight's `font-weight: bolder`, which computes to 700, and the
 * type system draws only 400 and 600. Arial draws both as bold, so nothing moves on screen and a
 * screenshot cannot tell them apart; the computed weight is the whole of the defect.
 * `components/privacy/privacyEmphasisWeight.browser.test.tsx` measures the shared privacy prose;
 * this file measures the four runs that live in the panel itself:
 *
 * - the enforcement-off strip's headline,
 * - the "Those three are not being stopped right now" line under the disclosure (tiers off),
 * - `every`, in the turn-off confirmation,
 * - the second consequence in that confirmation's list.
 *
 * The panel's look is presentation only (ruling 15): no copy, props, logic or test id moves, and
 * this test only reads computed styles.
 *
 * jsdom has no preflight and computes no cascade, so `PrivacyPanel.test.tsx` can only say which
 * class is on the element. This file bundles the real panel with esbuild (the config context, the
 * model context and the API client stubbed), compiles `styles/main.css` with Tailwind over the
 * classes the bundle uses, and reads `getComputedStyle`.
 *
 * The instrument is checked first: the page also carries one bare `<strong>`, and the first case
 * asserts it reads 700. A stylesheet without preflight would make every other case pass for the
 * wrong reason.
 *
 * To see these cases fail on another version of the panel, point `PRIVACY_PANEL_SOURCE` at a copy of
 * `PrivacyPanel.tsx`: it is loaded in place of the one beside this file, its imports resolved from
 * this directory.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../../package.json'));

const stubs: { filter: RegExp; contents: string }[] = [
  {
    // The daemon: the switch's value and its record come from the page, set before the bundle runs.
    filter: /\/ConfigContext$/,
    contents: `
      export const useConfig = () => ({
        read: async (key) => (globalThis.__privacy ?? {})[key],
        upsert: async () => {},
        getProviders: async () => [],
      });`,
  },
  {
    filter: /\/ModelAndProviderContext$/,
    contents: `
      export const useOptionalModelAndProvider = () => null;
      export const useModelAndProvider = () => ({ currentProvider: null, currentModel: null });`,
  },
  {
    // The served disclosure the panel renders in both toggle positions.
    filter: /\/api$/,
    contents: `
      export const getPrivacyDisclosure = async () => ({
        data: {
          title_template: '{provider} is not hosted by your institution.',
          long: globalThis.__disclosure,
          short: 'short',
          acknowledged: true,
        },
      });
      export const ackPrivacyDisclosure = async () => ({ data: undefined });`,
  },
  {
    filter: /\/utils\/userAction$/,
    contents: `export const userActionHeaders = async () => ({});`,
  },
];

const panelOverride = process.env.PRIVACY_PANEL_SOURCE;

const stubPlugin: Plugin = {
  name: 'stubs',
  setup(b) {
    for (const { filter, contents } of stubs) {
      b.onResolve({ filter }, (args) =>
        args.kind === 'import-statement'
          ? { path: args.path, namespace: 'stub', pluginData: contents }
          : undefined
      );
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      loader: 'js',
    }));
    if (panelOverride) {
      b.onResolve({ filter: /^\.\/PrivacyPanel$/ }, () => ({
        path: resolve(panelOverride),
        namespace: 'source-override',
      }));
      b.onLoad({ filter: /.*/, namespace: 'source-override' }, (args) => ({
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
import { MemoryRouter } from 'react-router-dom';
import PrivacyPanel from './PrivacyPanel';

createRoot(document.getElementById('root')!).render(
  <MemoryRouter>
    <div id="panel" className="mx-auto max-w-[760px] p-4 text-body text-text-default">
      <PrivacyPanel />
    </div>
    <p id="control" className="p-4 text-body">
      <strong id="bare">A bare strong, the instrument check</strong>
    </p>
  </MemoryRouter>
);
`;

/** The served long form, with the two emphasised clauses `DisclosureProse` draws as strong. */
const DISCLOSURE =
  'It can read **files on this computer**. Anything a chat on this model can reach, it can send there.\n\n' +
  'Biorouter does stop three things: another chat’s transcript, a private knowledge base and a private extension.\n\n' +
  'It **does not** stop it reading ordinary files through the shell.';

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

type Weights = {
  strong: { text: string; weight: string }[];
  /** The heaviest computed weight of anything inside the panel. */
  max: number;
};

describe('Settings, Privacy emphasis is 600, never 700 (V4-R7-6)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the privacy panel emphasis weight tests.');
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

  /** The panel with the switch `on` or `off`, in light or dark. */
  async function open(state: 'on' | 'off', dark = false): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 1100 } });
    await tab.setContent(
      `<!doctype html><html${dark ? ' class="dark"' : ''}><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.evaluate(
      ([value, disclosure]) => {
        Object.assign(globalThis, {
          __disclosure: disclosure,
          __privacy: { BIOROUTER_PRIVACY_TIERS: value },
        });
      },
      [state, DISCLOSURE]
    );
    await tab.addScriptTag({ content: script });
    // The switch is held until the daemon has answered, and the disclosure arrives after it.
    await tab.waitForSelector('[data-privacy-panel]:not([aria-busy="true"])');
    await tab.waitForSelector('[data-testid="non-private-model-statement"]');
    return tab;
  }

  const weights = (tab: Page): Promise<Weights> =>
    tab.evaluate(() => {
      const root = document.querySelector('#panel')!;
      const strong = [...root.querySelectorAll('strong')].map((el) => ({
        text: (el.textContent ?? '').replace(/\s+/g, ' ').trim(),
        weight: getComputedStyle(el).fontWeight,
      }));
      const all = [root, ...root.querySelectorAll('*')].map((el) =>
        parseInt(getComputedStyle(el).fontWeight, 10)
      );
      return { strong, max: Math.max(...all) };
    });

  const OFF_HEADLINE = 'Privacy tiers are off.';
  const NOT_STOPPED =
    'Those three are not being stopped right now. Privacy tiers are off on this machine.';
  const SECOND_CONSEQUENCE =
    'While it is off, Biorouter stops recording which chats touched private material.';

  it('reads a bare strong as 700, so the other cases are measuring something', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('on');
    try {
      const bare = await tab.evaluate(
        () => getComputedStyle(document.querySelector('#bare')!).fontWeight
      );
      expect(bare).toBe('700');
    } finally {
      await tab.close();
    }
  });

  for (const dark of [false, true]) {
    const mode = dark ? 'dark' : 'light';

    it(`draws the strip's headline and the not-stopped line at 600 with the tiers off (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open('off', dark);
      try {
        const w = await weights(tab);
        const texts = w.strong.map((s) => s.text);
        expect(texts).toContain(OFF_HEADLINE);
        expect(texts).toContain(NOT_STOPPED);
        // The disclosure's own two clauses are in the surface too, and are the same weight.
        expect(texts).toContain('files on this computer');
        for (const { text, weight } of w.strong) expect(weight, text).toBe('600');
        expect(w.max).toBeLessThanOrEqual(600);
      } finally {
        await tab.close();
      }
    });

    it(`draws "every" and the second consequence at 600 in the turn-off confirmation (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open('on', dark);
      try {
        await tab.getByRole('switch', { name: /Privacy tiers/ }).click();
        await tab.waitForSelector('[data-testid="privacy-disable-confirm"]');
        const w = await weights(tab);
        const texts = w.strong.map((s) => s.text);
        expect(texts).toContain('every');
        expect(texts).toContain(SECOND_CONSEQUENCE);
        for (const { text, weight } of w.strong) expect(weight, text).toBe('600');
        expect(w.max).toBeLessThanOrEqual(600);
      } finally {
        await tab.close();
      }
    });
  }
});
