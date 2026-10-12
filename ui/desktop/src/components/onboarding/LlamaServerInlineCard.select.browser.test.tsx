// @vitest-environment node
/**
 * The Llama Server card's Model picker, painted by a real engine (vision round 4, V4-R4-3).
 *
 * It was a native `<select>`: the OS chevron, heavier than the app's thin one and set 8px in, and
 * the OS popup menu, not the app's 32px rows with a trailing check. It is the shared `ui/Select`
 * now. jsdom has no layout and no native popup, so this test reads the real thing in Chromium:
 *
 * - the card holds no `<select>`, and the label still names the field (`for` -> `inputId`);
 * - the trigger is the 32px rung with the app's own 16px chevron, and the OS draws neither;
 * - a click opens the app's menu: portalled out of the card, one 32px-or-taller row per catalog
 *   model, the chosen one carrying the trailing check, and picking one moves the value;
 * - a start in flight disables it and dims it by the shared 50% step.
 *
 * The real `LlamaServerInlineCard` is bundled with esbuild (the API, the toasts and the config
 * context are stubbed), `styles/main.css` is compiled by Tailwind with the classes the bundle
 * uses, and `onboarding.css` is appended as authored. Set `WS_SCREENSHOT_DIR` to also save PNGs.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const ONBOARDING_CSS = readFileSync(resolve(here, 'onboarding.css'), 'utf8');

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import LlamaServerInlineCard from './LlamaServerInlineCard';

createRoot(document.getElementById('root')!).render(
  <div style={{ maxWidth: 560, margin: '48px auto' }}>
    <LlamaServerInlineCard onSuccess={() => {}} />
  </div>
);
`;

const entry = (name: string, display: string, size: string, extra = '') => `{
  name: '${name}', display_name: '${display}', family: 'Gemma 4', ollama_name: '${name}:latest',
  official_url: 'https://ollama.com/library/${name}', hf_spec: 'google/${name}:Q4_0',
  download_size: '${size}', description: 'A model', min_gpu_memory_gib: 16,
  recommended_gpu_memory_gib: 16, context_limit: 131072, active_params_b: 4,
  speed_hint: 'Fast', is_default: ${name === 'gemma4'}, downloaded: false,
  download_status: 'not_downloaded', download_source: 'none', fallback_downloaded: false,
  fallback_download_status: 'not_downloaded', model_path: null, suitable: true,
  suitability_status: 'suitable', suitability_message: 'Recommended for this machine.' ${extra}
}`;

/** What the card imports that is not its business: the API, the toasts and the config context. */
const STUBS: Record<string, string> = {
  api: `
    const status = () => ({
      data: {
        sidecar: { state: 'stopped', warmed: false, build: 'test', detail: null, model: null },
        catalog: [
          ${entry('gemma4', 'Gemma 4 E4B', '9.6 GB')},
          ${entry('gemma4-12b', 'Gemma 4 12B', '7.6 GB')},
          ${entry('qwen3.6-27b', 'Qwen3.6 27B', '17.1 GB')},
        ],
        system: { os: 'macos', total_memory_gib: 64, accelerator_memory_gib: 64,
                  accelerator_memory_kind: 'apple_unified', default_context_size: 131072,
                  model_cache_dir: '/tmp/models', model_cache_layout: 'test' },
      },
    });
    export const llamacppStatus = async () => status();
    // A start that never answers keeps the card "in flight" for as long as a test needs.
    export const llamacppEnsure = () => new Promise(() => {});
    export const llamacppWarmup = async () => ({ data: {} });
  `,
  toasts: `export const toastService = { error() {}, success() {} };`,
  ConfigContext: `const api = { upsert: async () => {} }; export const useConfig = () => api;`,
};

const stubPlugin: Plugin = {
  name: 'llama-card-stubs',
  setup(b) {
    b.onResolve({ filter: /(^|\/)(api|toasts)$|\/ConfigContext$/ }, (args) => {
      const key = args.path.endsWith('ConfigContext')
        ? 'ConfigContext'
        : args.path.endsWith('toasts')
          ? 'toasts'
          : 'api';
      return { path: key, namespace: 'stub' };
    });
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: STUBS[args.path],
      loader: 'tsx',
      resolveDir: here,
    }));
  },
};

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

interface Picker {
  nativeSelects: number;
  labelFor: string | null;
  triggerHeight: number;
  chevronSize: number;
  inputRole: string | null;
  labelName: string;
}

describe('the Llama Server card Model picker (V4-R4-3)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Llama Server select tests.');
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
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function openPage(mode: 'light' | 'dark'): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 900 } });
    tab.setDefaultTimeout(8000);
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css + ONBOARDING_CSS}</style></head>` +
        `<body style="margin:0" class="bg-background-canvas"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('input#llamacpp-model-select');
    await tab.waitForTimeout(300);
    return tab;
  }

  const measure = (tab: Page): Promise<Picker> =>
    tab.evaluate(() => {
      const input = document.getElementById('llamacpp-model-select') as HTMLInputElement;
      const field = document.querySelector('[data-testid="llamacpp-model-select"]') as HTMLElement;
      const control = field.querySelector('.br-select > div') as HTMLElement;
      const label = document.querySelector('label[for="llamacpp-model-select"]') as HTMLElement;
      const chevron = field.querySelector('svg') as SVGElement;
      return {
        nativeSelects: document.querySelectorAll('select').length,
        labelFor: label.getAttribute('for'),
        triggerHeight: Math.round(control.getBoundingClientRect().height),
        chevronSize: Math.round(chevron.getBoundingClientRect().width),
        inputRole: input.getAttribute('role'),
        labelName: label.textContent ?? '',
      };
    });

  const shot = async (tab: Page, name: string) => {
    const dir = process.env.WS_SCREENSHOT_DIR;
    if (!dir) return;
    mkdirSync(dir, { recursive: true });
    await tab.screenshot({ path: resolve(dir, `${name}.png`) });
  };

  for (const mode of ['light', 'dark'] as const) {
    it(`draws the app's own trigger, not the OS's, in ${mode}`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage(mode);
      try {
        const picker = await measure(tab);
        expect(picker.nativeSelects, 'no native <select> in the card').toBe(0);
        expect(picker.labelFor).toBe('llamacpp-model-select');
        expect(picker.labelName).toBe('Model');
        expect(picker.inputRole).toBe('combobox');
        expect(picker.triggerHeight, 'the 32px rung').toBe(32);
        expect(picker.chevronSize, "the app's 16px chevron").toBe(16);
        await shot(tab, `llama-select-closed-${mode}`);
      } finally {
        await tab.close();
      }
    });

    it(`opens the app's menu with a check on the chosen row, and picks, in ${mode}`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage(mode);
      try {
        await tab.click('[data-testid="llamacpp-model-select"] .br-select > div');
        await tab.waitForSelector('.br-select-menu');
        // The menu opens at scale .97 and settles over `--dur-fast-max`: measure the settled box.
        await tab.waitForTimeout(400);
        const menu = await tab.evaluate(() => {
          const el = document.querySelector('.br-select-menu') as HTMLElement;
          const field = document.querySelector(
            '[data-testid="llamacpp-model-select"]'
          ) as HTMLElement;
          const control = field.querySelector('.br-select > div') as HTMLElement;
          const rows = [...el.querySelectorAll('[role="option"]')] as HTMLElement[];
          return {
            portalled: !document.getElementById('root')!.contains(el),
            rows: rows.map((row) => ({
              text: row.textContent ?? '',
              height: Math.round(row.getBoundingClientRect().height),
              check: !!row.querySelector('[data-slot="menu-item-indicator"]'),
            })),
            gap: Math.round(
              el.getBoundingClientRect().top - control.getBoundingClientRect().bottom
            ),
          };
        });
        expect(menu.portalled, 'the menu is not clipped by the card').toBe(true);
        expect(menu.rows.map((row) => row.text)).toEqual([
          'Gemma 4 E4B · 9.6 GB · Needs download',
          'Gemma 4 12B · 7.6 GB · Needs download',
          'Qwen3.6 27B · 17.1 GB · Needs download',
        ]);
        expect(
          menu.rows.every((row) => row.height >= 32),
          'every row is at least the 32px rung'
        ).toBe(true);
        expect(menu.rows.map((row) => row.check)).toEqual([true, false, false]);
        expect(menu.gap, 'it hangs just under the trigger').toBeLessThan(12);
        await shot(tab, `llama-select-open-${mode}`);

        await tab.click('[role="option"]:has-text("Gemma 4 12B")');
        await tab.waitForSelector('.br-select-menu', { state: 'detached' });
        const value = await tab.textContent('[data-testid="llamacpp-model-select"]');
        expect(value).toContain('Gemma 4 12B · 7.6 GB · Needs download');
        // The facts below follow the pick: the size moves to the button, never a third copy in
        // the Details summary (FR-R2-6).
        expect(await tab.textContent('[data-testid="llamacpp-start"]')).toContain('7.6 GB');
        expect(await tab.textContent('[data-testid="llamacpp-size-speed"]')).not.toContain('GB');
      } finally {
        await tab.close();
      }
    });
  }

  it('is disabled and dimmed by the shared 50% step while a start is in flight', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage('light');
    try {
      await tab.click('[data-testid="llamacpp-start"]');
      await tab.waitForSelector('[data-testid="llamacpp-progress"]');
      // The control fades to its disabled step over a short transition: measure the settled paint.
      await tab.waitForTimeout(400);
      const state = await tab.evaluate(() => {
        const input = document.getElementById('llamacpp-model-select') as HTMLInputElement;
        const control = document.querySelector(
          '[data-testid="llamacpp-model-select"] .br-select > div'
        ) as HTMLElement;
        return { disabled: input.disabled, opacity: getComputedStyle(control).opacity };
      });
      expect(state.disabled).toBe(true);
      expect(state.opacity).toBe('0.5');
    } finally {
      await tab.close();
    }
  });
});
