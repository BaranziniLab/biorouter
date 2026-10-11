// @vitest-environment node
/**
 * The Add provider form, laid out by a real engine.
 *
 * - Vision V4-R4-5: the two checkbox squares ("This is a local model", "Provider supports streaming
 *   responses") started 4px inside the form's left edge, because the Checkbox centres its 16px
 *   square in a 24px hit target. Each row now moves back 4px (`.br-provider-check-row`), so the
 *   squares share the edge the labels, the fields and the Select trigger start on.
 *
 * jsdom computes no layout, so the real `CustomProviderForm` is bundled with esbuild, `main.css` is
 * compiled by Tailwind with the classes the bundle uses, `providers.css` is appended as authored,
 * and Chromium measures it. An instrument case removes the row's shift and reads the old 4px, so
 * the measurement is shown able to fail. Set `WS_SCREENSHOT_DIR` to also save PNGs.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../../package.json'));
const PROVIDERS_CSS = readFileSync(resolve(here, 'providers.css'), 'utf8');

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import CustomProviderForm from './modal/subcomponents/forms/CustomProviderForm';

createRoot(document.getElementById('root')!).render(
  <div style={{ width: 448, padding: 16, background: 'var(--background-default)' }}>
    <CustomProviderForm initialData={null} isEditable onSubmit={() => {}} onCancel={() => {}} />
  </div>
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

interface Edges {
  labels: number[];
  fields: number[];
  select: number;
  squares: number[];
}

describe('the Add provider form (V4-R4-5)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Add provider form tests.');
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
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(mode: 'light' | 'dark', extraCss = ''): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 640, height: 900 } });
    tab.setDefaultTimeout(8000);
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${
        css + PROVIDERS_CSS + extraCss
      }</style></head><body style="margin:0" class="bg-background-default"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#supports-streaming');
    return tab;
  }

  const edges = (tab: Page): Promise<Edges> =>
    tab.evaluate(() => {
      const left = (el: Element | null) => (el as HTMLElement).getBoundingClientRect().left;
      const form = document.querySelector('form') as HTMLFormElement;
      return {
        labels: Array.from(form.querySelectorAll('label[for]')).map(left),
        fields: Array.from(form.querySelectorAll('input:not([type="checkbox"])'))
          .filter((input) => (input as HTMLElement).offsetParent !== null)
          .filter((input) => !(input as HTMLElement).closest('.br-select'))
          .map((input) => left((input as HTMLElement).closest('.br-field') ?? input)),
        select: left(form.querySelector('.br-select > div')),
        squares: ['custom-provider-local-row', 'custom-provider-streaming-row'].map((id) =>
          left(document.querySelector(`[data-testid="${id}"] .br-checkbox-box`))
        ),
      };
    });

  const shot = async (tab: Page, name: string) => {
    const dir = process.env.WS_SCREENSHOT_DIR;
    if (!dir) return;
    mkdirSync(dir, { recursive: true });
    await tab.screenshot({ path: resolve(dir, `${name}.png`) });
  };

  for (const mode of ['light', 'dark'] as const) {
    it(`starts both checkbox squares on the labels' and fields' edge in ${mode}`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode);
      try {
        const at = await edges(tab);
        const edge = at.labels[0];
        expect(at.labels.length).toBeGreaterThanOrEqual(4);
        for (const x of [...at.labels, ...at.fields, at.select]) expect(x).toBeCloseTo(edge, 1);
        expect(at.squares).toHaveLength(2);
        for (const x of at.squares) expect(x, 'checkbox square left edge').toBeCloseTo(edge, 1);
        await shot(tab, `add-provider-form-${mode}`);
      } finally {
        await tab.close();
      }
    });
  }

  it('instrument: without the row shift the squares sit 4px inside the edge', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light', '.br-provider-check-row { margin-inline-start: 0 }');
    try {
      const at = await edges(tab);
      for (const x of at.squares) expect(x - at.labels[0]).toBeCloseTo(4, 1);
    } finally {
      await tab.close();
    }
  });
});
