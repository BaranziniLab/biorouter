// @vitest-environment node
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
const styles = resolve(here, '../../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../../package.json'));
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ModeSection } from './ModeSection';
createRoot(document.getElementById('root')!).render(
  <main style={{ padding: 16 }}><ModeSection /></main>
);
`;
const fixtureConfig: Plugin = {
  name: 'isolated-settings-config',
  setup(builder) {
    builder.onResolve({ filter: /\/ConfigContext$/ }, () => ({
      path: 'config',
      namespace: 'fixture',
    }));
    builder.onResolve({ filter: /\/PermissionRulesModal$/ }, () => ({
      path: 'permissions',
      namespace: 'fixture',
    }));
    builder.onLoad({ filter: /.*/, namespace: 'fixture' }, ({ path }) => ({
      contents:
        path === 'permissions'
          ? 'export default () => null;'
          : `
        const values = { BIOROUTER_MODE: 'auto', BIOROUTER_MAX_TURNS: Number(new URLSearchParams(location.search).get('turns') || 0) };
        const api = {
          read: async (key) => values[key],
          upsert: async (key, value) => { values[key] = value; },
        };
        export const useConfig = () => api;
      `,
      loader: 'js',
    }));
  },
};

async function launch(): Promise<Browser | null> {
  for (const options of [{}, { channel: 'chromium' }, { channel: 'chrome' }] as const) {
    try {
      return await chromium.launch(options);
    } catch {
      /* Try the next installed browser. */
    }
  }
  return null;
}

describe('Max turns in the production settings layout', () => {
  let browser: Browser | null;
  let script: string;
  let css: string;

  beforeAll(async () => {
    browser = await launch();
    if (!browser) return;
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"' },
      plugins: [fixtureConfig],
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(styles, 'main.css'), 'utf8'), {
      base: styles,
      onDependency() {},
    });
    css = (
      await postcss([fontScale()]).process(
        compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])),
        { from: undefined }
      )
    ).css;
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  });

  async function open(width: number, zoom: number): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width, height: 900 } });
    await page.setContent(
      `<!doctype html><html style="zoom:${zoom}"><head><style>${css}</style></head><body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: script });
    await page.getByRole('radio', { name: 'Unlimited', exact: true }).waitFor();
    return page;
  }

  async function geometry(page: Page) {
    return page.locator('.br-max-turns-row').evaluate((row) => {
      const rect = (element: Element) => {
        const r = element.getBoundingClientRect();
        return {
          left: r.left,
          right: r.right,
          top: r.top,
          bottom: r.bottom,
          height: r.height,
          mid: r.top + r.height / 2,
        };
      };
      return {
        row: rect(row),
        label: rect(row.querySelector('.br-setting-row-label')!),
        selector: rect(row.querySelector('[role="radiogroup"]')!),
        input: row.querySelector('input[type="number"]')
          ? rect(row.querySelector('input[type="number"]')!)
          : null,
        turnsLabel: row.querySelector('.br-max-turns-input label')
          ? rect(row.querySelector('.br-max-turns-input label')!)
          : null,
        horizontalOverflow: document.documentElement.scrollWidth > innerWidth,
        valueSlots: row.querySelectorAll('.br-setting-row-value').length,
      };
    });
  }

  it('aligns the mode selector, input and label and keeps errors from moving the controls', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const zoom of [1, 2]) {
      const page = await open(zoom * 760, zoom);
      try {
        const unlimited = await geometry(page);
        expect(unlimited.valueSlots).toBe(0);
        expect(unlimited.input).toBeNull();
        expect(unlimited.selector.mid).toBeCloseTo(unlimited.label.mid, 1);
        await page.getByRole('radio', { name: 'Set a limit' }).click();
        await page.getByRole('spinbutton', { name: 'Turns' }).fill('40');
        const finite = await geometry(page);
        expect(finite.input!.height).toBeCloseTo(finite.selector.height, 1);
        expect(finite.input!.mid).toBeCloseTo(finite.selector.mid, 1);
        expect(finite.turnsLabel!.mid).toBeCloseTo(finite.input!.mid, 1);
        expect(finite.label.mid).toBeCloseTo(finite.selector.mid, 1);
        await page.getByRole('spinbutton', { name: 'Turns' }).fill('');
        await page.getByRole('alert').waitFor();
        const invalid = await geometry(page);
        expect(invalid.selector.top).toBeCloseTo(finite.selector.top, 1);
        expect(invalid.input!.top).toBeCloseTo(finite.input!.top, 1);
        expect(invalid.horizontalOverflow).toBe(false);
      } finally {
        await page.close();
      }
    }
  });

  it('keeps every control visible without overflow at narrow widths and 200% zoom', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const [width, zoom] of [
      [360, 1],
      [720, 2],
      [640, 2],
    ]) {
      const page = await open(width, zoom);
      try {
        await page.getByRole('radio', { name: 'Set a limit' }).click();
        await page.getByRole('alert').waitFor();
        const boxes = await geometry(page);
        expect(boxes.horizontalOverflow).toBe(false);
        expect(boxes.input!.height).toBeCloseTo(boxes.selector.height, 1);
        expect(boxes.turnsLabel!.mid).toBeCloseTo(boxes.input!.mid, 1);
        for (const box of [boxes.label, boxes.selector, boxes.input!]) {
          expect(box.left).toBeGreaterThanOrEqual(boxes.row.left);
          expect(box.right).toBeLessThanOrEqual(boxes.row.right);
        }
        expect(boxes.label.bottom).toBeLessThanOrEqual(boxes.selector.top);
        await page.getByRole('spinbutton', { name: 'Turns' }).fill('4294967295');
        await page.getByRole('radio', { name: 'Unlimited', exact: true }).click();
        await page.getByRole('spinbutton').waitFor({ state: 'detached' });
        await expect(
          page.getByRole('radio', { name: 'Unlimited', exact: true }).getAttribute('aria-checked')
        ).resolves.toBe('true');
      } finally {
        await page.close();
      }
    }
  });
});
