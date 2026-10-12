// @vitest-environment node
/**
 * The emphasised words in two of WS-PRIMITIVES' dialogs are 600, not 700 (vision V4-R7-6;
 * principle 3), measured in Chromium with the whole stylesheet:
 *
 * - the workflow warning's preview names its fields ("Title", "Description"),
 * - the update dialog, once the download is done, names the button it points at ("Restart &
 *   update").
 *
 * A bare `<strong>` takes Tailwind preflight's `font-weight: bolder`, which computes to 700 on
 * Arial, and the type system draws only 400 and 600. Arial draws both as bold, so nothing moves on
 * screen and a screenshot cannot tell them apart; the computed weight is the whole of the defect.
 * jsdom has no preflight and computes no cascade, so the two component tests can only say which
 * class is on the element. This file bundles the real `WorkflowWarningModal` (its markdown renderer
 * stubbed) and `UpdateAvailableModal` (the Electron bridge stubbed) with esbuild, compiles
 * `styles/main.css` with Tailwind over the classes the bundle uses, and reads `getComputedStyle`.
 *
 * ⚠ The instrument is checked first: the page also carries one bare `<strong>`, and the first case
 * asserts it reads 700. A stylesheet without preflight would make every other case pass for the
 * wrong reason.
 *
 * To see these cases fail on another version of the components, point `MODAL_EMPHASIS_SOURCE_DIR`
 * at a directory holding that version's `WorkflowWarningModal.tsx` and `UpdateAvailableModal.tsx`:
 * they are loaded in place of the real ones, their imports resolved from where those live.
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
const COMPONENTS_DIR = resolve(here, '..');
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

/** Each component, the specifier the entry imports it by, and the folder its imports resolve in. */
const SOURCES = [
  { file: 'WorkflowWarningModal.tsx', specifier: './WorkflowWarningModal', dir: here },
  { file: 'UpdateAvailableModal.tsx', specifier: '../UpdateAvailableModal', dir: COMPONENTS_DIR },
];

const stubPlugin: Plugin = {
  name: 'stubs',
  setup(b) {
    // The preview's instructions render as markdown; the weight under test is not in them.
    b.onResolve({ filter: /^\.\.\/MarkdownContent$/ }, () => ({
      path: 'MarkdownContent',
      namespace: 'stub',
    }));
    b.onLoad({ filter: /.*/, namespace: 'stub' }, () => ({
      contents: `export default function MarkdownContent({ content }) {
        return React.createElement('div', null, content);
      }
      import * as React from 'react';`,
      loader: 'js',
      resolveDir: here,
    }));
    const dir = process.env.MODAL_EMPHASIS_SOURCE_DIR;
    if (dir) {
      for (const source of SOURCES) {
        const filter = new RegExp(`^${source.specifier.replace(/[.\\/]/g, '\\$&')}$`);
        b.onResolve({ filter }, () => ({
          path: resolve(dir, source.file),
          namespace: 'source-override',
          pluginData: source.dir,
        }));
      }
      b.onLoad({ filter: /.*/, namespace: 'source-override' }, (args) => ({
        contents: readFileSync(args.path, 'utf8'),
        resolveDir: args.pluginData as string,
        loader: 'tsx',
      }));
    }
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { WorkflowWarningModal } from './WorkflowWarningModal';
import UpdateAvailableModal from '../UpdateAvailableModal';

const scene = (globalThis as { __scene?: string }).__scene;

createRoot(document.getElementById('root')!).render(
  <>
    {scene === 'workflow' ? (
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={{
          title: 'Cohort summary',
          description: 'Summarizes a cohort.',
          instructions: 'Read the table.',
        }}
      />
    ) : (
      <UpdateAvailableModal />
    )}
    <p id="control" className="p-4 text-body">
      <strong id="bare">A bare strong, the instrument check</strong>
    </p>
  </>
);
`;

/** What the Electron preload exposes, as far as the update dialog reads it. */
const ELECTRON_STUB = `
  window.electron = {
    getVersion: () => '1.85.4',
    onUpdaterEvent: () => () => {},
    getUpdateState: () => Promise.resolve(null),
    installUpdate: () => {},
    openExternal: () => Promise.resolve(),
  };
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
  /** The heaviest computed weight of anything inside the dialog. */
  max: number;
};

describe('Dialog emphasis is 600, never 700 (V4-R7-6)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the dialog emphasis weight tests.');
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

  async function open(scene: 'workflow' | 'update', dark = false): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1440, height: 900 } });
    await tab.setContent(
      `<!doctype html><html${dark ? ' class="dark"' : ''}><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({
      content: `${ELECTRON_STUB}\nglobalThis.__scene = ${JSON.stringify(scene)};`,
    });
    await tab.addScriptTag({ content: script });
    if (scene === 'update') {
      // The sidebar's Update row asks for the dialog with the state it holds: downloaded. The
      // dialog listens from an effect, which React runs after the script returns, so the request is
      // repeated until the dialog is up rather than sent once into a page that may not listen yet.
      await tab.waitForFunction(() => {
        if (document.querySelector('[role="dialog"]')) return true;
        window.dispatchEvent(
          new CustomEvent('biorouter:open-update-modal', {
            detail: {
              phase: 'downloaded',
              latestVersion: '1.86.0',
              percent: 100,
              usingFallback: false,
            },
          })
        );
        return false;
      });
    }
    await tab.waitForSelector('[role="dialog"] strong');
    return tab;
  }

  const weightsIn = (tab: Page): Promise<Weights> =>
    tab.evaluate(() => {
      const panel = document.querySelector('[role="dialog"]')!;
      const strong = [...panel.querySelectorAll('strong')].map((el) => ({
        text: el.textContent,
        weight: getComputedStyle(el).fontWeight,
      }));
      const all = [panel, ...panel.querySelectorAll('*')].map((el) =>
        parseInt(getComputedStyle(el).fontWeight, 10)
      );
      return { strong, max: Math.max(...all) };
    });

  it('the instrument: a bare strong on the same page reads 700', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('workflow');
    try {
      const weight = await tab.evaluate(
        () => getComputedStyle(document.getElementById('bare')!).fontWeight
      );
      expect(weight).toBe('700');
    } finally {
      await tab.close();
    }
  });

  for (const dark of [false, true]) {
    const mode = dark ? 'dark' : 'light';

    it(`the workflow warning names its fields at 600, nothing heavier (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open('workflow', dark);
      try {
        const weights = await weightsIn(tab);
        expect(weights.strong.map((s) => s.text)).toEqual(['Title', 'Description']);
        for (const { text, weight } of weights.strong) expect(weight, text ?? '').toBe('600');
        expect(weights.max).toBeLessThanOrEqual(600);
      } finally {
        await tab.close();
      }
    });

    it(`the update dialog names its button at 600, nothing heavier (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open('update', dark);
      try {
        const weights = await weightsIn(tab);
        expect(weights.strong.map((s) => s.text)).toEqual(['Restart & update']);
        expect(weights.strong[0].weight).toBe('600');
        expect(weights.max).toBeLessThanOrEqual(600);
      } finally {
        await tab.close();
      }
    });
  }
});
