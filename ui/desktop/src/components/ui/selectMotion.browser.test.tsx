// @vitest-environment node
/**
 * The shared Select's trigger moves like the <Input> it copies (first run, FR-R3-5): colour,
 * background and border over `--dur-fast` on `--ease-out`, never `transition: all`.
 *
 * react-select emits its base control style even when `unstyled`, and it carries an unlayered
 * Emotion `transition: all 100ms` that beat the authored `transition-[color,background-color,
 * border-color]` (Tailwind's utilities layer): the Llama Model field computed `all 0.1s`, and its
 * chevron container `color 0.15s`. Spec 2.6 rules out `transition-all` and off-token durations.
 *
 * Emotion injects its rules at runtime, so this reads computed styles in Chromium with
 * `styles/main.css` compiled by Tailwind. The first case renders a bare `unstyled` react-select with
 * the same class and expects the library's values, so the others cannot pass vacuously.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import ReactSelect from 'react-select';
import { Select } from './Select';

const options = [
  { value: 'gemma4', label: 'Gemma 4' },
  { value: 'qwen', label: 'Qwen 3.6' },
];

function App() {
  return (
    <div style={{ padding: 24, display: 'grid', gap: 24, width: 320 }}>
      <Select
        classNamePrefix="shared"
        aria-label="Model"
        options={options}
        defaultValue={options[0]}
        isClearable
      />
      <ReactSelect
        unstyled
        classNamePrefix="bare"
        aria-label="Bare"
        options={options}
        defaultValue={options[0]}
        isClearable
        classNames={{ control: () => 'transition-[color,background-color,border-color]' }}
      />
    </div>
  );
}
createRoot(document.getElementById('root')!).render(<App />);
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

interface Motion {
  property: string;
  duration: string;
  timing: string;
}

describe('the shared Select moves on the motion tokens (FR-R3-5)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Select motion tests.');
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

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 600, height: 400 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('.shared__control');
    await tab.waitForSelector('.bare__control');
    return tab;
  }

  const motion = (tab: Page, selector: string) =>
    tab.evaluate((sel): Motion => {
      const cs = getComputedStyle(document.querySelector(sel) as HTMLElement);
      return {
        property: cs.transitionProperty,
        duration: cs.transitionDuration,
        timing: cs.transitionTimingFunction,
      };
    }, selector);

  /** `--dur-fast` and `--ease-out` as the page resolves them, so the test follows the tokens. */
  const tokens = (tab: Page) =>
    tab.evaluate(() => {
      const root = getComputedStyle(document.documentElement);
      const ms = parseFloat(root.getPropertyValue('--dur-fast'));
      return { fast: `${ms / 1000}s`, ease: root.getPropertyValue('--ease-out').trim() };
    });

  it("animates every property over 100ms without the override (react-select's base wins)", async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      expect(await motion(tab, '.bare__control')).toMatchObject({
        property: 'all',
        duration: '0.1s',
      });
      expect(await motion(tab, '.bare__dropdown-indicator')).toMatchObject({
        property: 'color',
        duration: '0.15s',
      });
    } finally {
      await tab.close();
    }
  });

  it("moves the trigger's colour, background and border over --dur-fast on --ease-out", async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const { fast, ease } = await tokens(tab);
      expect(fast).toBe('0.125s');
      const control = await motion(tab, '.shared__control');
      expect(control.property).toBe('color, background-color, border-color');
      expect(control.duration).toBe([fast, fast, fast].join(', '));
      expect(control.timing).toBe([ease, ease, ease].join(', '));
      expect(control.property).not.toMatch(/\ball\b/);
    } finally {
      await tab.close();
    }
  });

  it('moves the chevron and the clear mark on the same tokens', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const { fast, ease } = await tokens(tab);
      for (const selector of ['.shared__dropdown-indicator', '.shared__clear-indicator']) {
        expect(await motion(tab, selector), selector).toEqual({
          property: 'color',
          duration: fast,
          timing: ease,
        });
      }
    } finally {
      await tab.close();
    }
  });
});
