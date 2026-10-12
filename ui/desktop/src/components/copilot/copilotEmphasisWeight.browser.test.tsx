// @vitest-environment node
/**
 * The Biorouter Copilot panel names the computer at 600, not 700 (vision V4-R7-6, principle 3),
 * measured in Chromium with the whole stylesheet.
 *
 * `Computer: <strong>{target}</strong>` was a bare `<strong>`, which takes Tailwind preflight's
 * `font-weight: bolder` and computes to 700 on Arial; the type system draws only 400 and 600.
 * Arial draws both as bold, so a screenshot cannot tell them apart and the computed weight is the
 * whole of the defect. jsdom has no preflight and computes no cascade, so
 * `CopilotControl.test.tsx` can only pin the class; this file bundles the real component with
 * esbuild (its daemon calls and the surface probe stubbed), compiles `styles/main.css` with
 * Tailwind over the classes the bundle uses, and reads `getComputedStyle`.
 *
 * ⚠ The instrument is checked first: the page also carries one bare `<strong>`, and the first case
 * asserts it reads 700. A stylesheet without preflight would make the other cases pass for the
 * wrong reason. Same pattern as `privacy/privacyEmphasisWeight.browser.test.tsx`.
 *
 * To see the cases fail on another version of the component, point `COPILOT_SOURCE` at that
 * version's `CopilotControl.tsx`: it is loaded in place of the one beside this file, its imports
 * resolved from this directory.
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

/** A pending public request: the details are open, so the computer name is on screen. */
const STATUS = {
  session_id: 'task-a',
  provider: 'public-provider',
  model: 'model-a',
  destination: 'https://provider.example/api',
  target: 'backend-host',
  disclosure: '',
  state: 'approval_required',
  challenge_id: 'challenge-a',
  public_model: true,
  handoff_required: false,
  requested: true,
  enabled: true,
  activity_id: 'activity-a',
};

const STUBS: { filter: RegExp; contents: string }[] = [
  {
    filter: /^\.\/copilotApi$/,
    contents: `
      const STATUS = ${JSON.stringify(STATUS)};
      export class CopilotNotApplicable extends Error {}
      export const copilotStatus = async () => STATUS;
      export const copilotDecision = async () => STATUS;
      export const copilotSetup = async () => ({ status: 'ready', permissions: {} });`,
  },
  {
    filter: /^\.\.\/\.\.\/utils\/surface$/,
    contents: `export const isBrowserSurface = () => false;`,
  },
];

const stubPlugin: Plugin = {
  name: 'stubs',
  setup(b) {
    for (const { filter, contents } of STUBS) {
      b.onResolve({ filter }, (args) => ({
        path: args.path,
        namespace: 'stub',
        pluginData: contents,
      }));
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      loader: 'js',
    }));
    const source = process.env.COPILOT_SOURCE;
    if (source) {
      b.onResolve({ filter: /^\.\/CopilotControl$/ }, () => ({
        path: resolve(source),
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
import { CopilotControl } from './CopilotControl';

createRoot(document.getElementById('root')!).render(
  <>
    <div id="panel" className="text-body">
      <CopilotControl sessionId="task-a" />
    </div>
    <p id="control" className="p-4 text-body">
      <strong id="bare">A bare strong, the instrument check</strong>
    </p>
  </>
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

type Weights = {
  strong: { text: string | null; weight: string }[];
  /** The heaviest computed weight of anything inside the container. */
  max: number;
};

describe('The Copilot panel names the computer at 600, never 700 (V4-R7-6)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Copilot emphasis weight tests.');
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

  async function open(dark = false): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 900 } });
    await tab.setContent(
      `<!doctype html><html${dark ? ' class="dark"' : ''}><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    // The panel renders once the stubbed status read answers.
    await tab.waitForSelector('section[aria-label="Biorouter Copilot"] strong');
    return tab;
  }

  const weights = (tab: Page, selector: string): Promise<Weights> =>
    tab.evaluate((sel) => {
      const root = document.querySelector(sel)!;
      const strong = [...root.querySelectorAll('strong')].map((el) => ({
        text: el.textContent,
        weight: getComputedStyle(el).fontWeight,
      }));
      const all = [root, ...root.querySelectorAll('*')].map((el) =>
        parseInt(getComputedStyle(el).fontWeight, 10)
      );
      return { strong, max: Math.max(...all) };
    }, selector);

  it('reads a bare strong as 700, so the other cases are measuring something', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const control = await weights(tab, '#control');
      expect(control.strong.map((s) => s.weight)).toEqual(['700']);
    } finally {
      await tab.close();
    }
  });

  for (const dark of [false, true]) {
    const mode = dark ? 'dark' : 'light';

    it(`draws the computer name at 600 and nothing in the panel heavier (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(dark);
      try {
        const w = await weights(tab, 'section[aria-label="Biorouter Copilot"]');
        expect(w.strong.map((s) => s.text)).toEqual([STATUS.target]);
        for (const { text, weight } of w.strong) expect(weight, text ?? '').toBe('600');
        expect(w.max).toBeLessThanOrEqual(600);
      } finally {
        await tab.close();
      }
    });
  }
});
