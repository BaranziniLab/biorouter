// @vitest-environment node
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const styles = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ModalShell } from '../ModalShell';
import { Select } from './Select';

const options = Array.from({ length: 30 }, (_, index) => ({
  value: 'gemini-' + index,
  label: 'gemini-' + index,
}));

function App() {
  const [model, setModel] = React.useState(options[0]);
  return (
    <ModalShell open onOpenChange={() => {}} purpose="form" scrollBody title="Switch models"
      subtitle="Select a provider and model for this chat.">
      <div style={{ height: 150, paddingTop: 70 }}>
        <Select aria-label="Model" classNamePrefix="model" value={model} onChange={setModel}
          options={[{ options }]} menuPortalTarget={document.body} menuPlacement="auto" />
        <output id="selected">{model.label}</output>
      </div>
    </ModalShell>
  );
}

createRoot(document.getElementById('root')).render(<App />);
`;

async function launchChromium(): Promise<Browser | null> {
  for (const options of [{}, { channel: 'chromium' }, { channel: 'chrome' }] as const) {
    try {
      return await chromium.launch(options);
    } catch {
      // Try the next available browser.
    }
  }
  return null;
}

describe('a model menu inside a dialog', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) return;
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
    const compiler = await compile(readFileSync(resolve(styles, 'main.css'), 'utf8'), {
      base: styles,
      onDependency() {},
    });
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  });

  async function openMenu(): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width: 900, height: 900 } });
    await page.setContent(
      `<!doctype html><html><head><style>${css}</style></head>` +
        '<body style="min-height: 2000px"><div id="root"></div></body></html>'
    );
    await page.addScriptTag({ content: script });
    await page.locator('.model__control').click();
    await page.locator('[role="listbox"]').waitFor();
    await page.waitForTimeout(400);
    return page;
  }

  it('scrolls with small continuous wheel deltas and ordinary wheel steps', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await openMenu();
    try {
      const list = page.locator('[role="listbox"]');
      const geometry = await list.evaluate((element) => ({
        insideDialog: Boolean(element.closest('[role="dialog"]')),
        clientHeight: element.clientHeight,
        scrollHeight: element.scrollHeight,
      }));
      expect(geometry.insideDialog).toBe(false);
      expect(geometry.scrollHeight).toBeGreaterThan(geometry.clientHeight);
      await list.hover();

      for (let index = 0; index < 12; index += 1) await page.mouse.wheel(0, 2.5);
      await expect.poll(() => list.evaluate((element) => element.scrollTop)).toBeGreaterThan(20);
      const afterSmallSteps = await list.evaluate((element) => element.scrollTop);

      await page.mouse.wheel(0, 120);
      await expect
        .poll(() => list.evaluate((element) => element.scrollTop))
        .toBeGreaterThan(afterSmallSteps + 90);
      const afterWheel = await list.evaluate((element) => element.scrollTop);

      for (let index = 0; index < 12; index += 1) await page.mouse.wheel(0, -2.5);
      await expect
        .poll(() => list.evaluate((element) => element.scrollTop))
        .toBeLessThan(afterWheel - 20);
      expect(await page.locator('#selected').textContent()).toBe('gemini-0');

      await list.evaluate((element) => {
        element.scrollTop = element.scrollHeight;
      });
      await page.mouse.wheel(0, 120);
      await page.waitForTimeout(100);
      expect(await page.evaluate(() => window.scrollY)).toBe(0);
    } finally {
      await page.close();
    }
  });

  it('still chooses a model with the keyboard', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await openMenu();
    try {
      await page.keyboard.press('ArrowDown');
      await page.keyboard.press('Enter');
      await expect.poll(() => page.locator('#selected').textContent()).toBe('gemini-1');
    } finally {
      await page.close();
    }
  });
});
