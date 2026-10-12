// @vitest-environment node
/**
 * The local-model warm-up dialog, measured in Chromium with the whole stylesheet (vision round 7,
 * V3-R7-1; spec 2.6). The jsdom file beside it pins the construction; jsdom lays nothing out, so
 * the properties that were the reason for that construction are read here: where the dialog sits,
 * that its own buttons stay in view when the window is short and the detail is long, that it is
 * the detail that scrolls, and what the filled panel and the spinner paint.
 *
 * The real `ModelAndProviderProvider` is bundled with esbuild. Only its two outside edges are
 * stubbed: `../api` keeps every real export and overrides the four calls the switch makes, and
 * `./ConfigContext` keeps its exports and hands the provider a stable `useConfig`. The dialog is
 * opened the way the picker opens it (`changeModel` onto a Llama Server model). `styles/main.css`
 * is compiled by Tailwind over the classes the bundle uses and the renderer's text-size pass runs.
 * Set `WARMUP_SHOTS_DIR` to also save screenshots there.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../styles');
const requireFromDesktop = createRequire(resolve(here, '../../package.json'));
const SHOTS = process.env.WARMUP_SHOTS_DIR;

const API_ENTRY = resolve(here, '../api/index.ts');
const CONFIG_CONTEXT = resolve(here, './ConfigContext.tsx');

const stubs: Plugin = {
  name: 'warmup-stubs',
  setup(b) {
    b.onResolve({ filter: /^\.\.\/api$/ }, () => ({ path: 'api-stub', namespace: 'stub' }));
    b.onResolve({ filter: /^\.\/ConfigContext$/ }, () => ({
      path: 'config-stub',
      namespace: 'stub',
    }));
    b.onLoad({ filter: /^api-stub$/, namespace: 'stub' }, () => ({
      loader: 'ts',
      resolveDir: here,
      contents: `
        export * from ${JSON.stringify(API_ENTRY)};
        export const llamacppStatus = async () => ({ data: window.__status });
        export const llamacppWarmup = () =>
          window.__hang
            ? new Promise(() => {})
            : Promise.resolve({ data: { output: 'pong', sidecar: window.__status.sidecar } });
        export const setConfigProvider = async () => undefined;
        export const updateAgentProvider = async () => undefined;
      `,
    }));
    b.onLoad({ filter: /^config-stub$/, namespace: 'stub' }, () => ({
      loader: 'ts',
      resolveDir: here,
      contents: `
        export * from ${JSON.stringify(CONFIG_CONTEXT)};
        const stable = {
          read: async (key) =>
            key === 'BIOROUTER_MODEL' ? 'gpt-4o' : key === 'BIOROUTER_PROVIDER' ? 'openai' : null,
          getProviders: async () => [],
          refreshConfig: async () => undefined,
        };
        export const useConfig = () => stable;
      `,
    }));
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ModelAndProviderProvider, useModelAndProvider } from './ModelAndProviderContext';

function Open() {
  const { changeModel } = useModelAndProvider();
  const started = React.useRef(false);
  React.useEffect(() => {
    if (started.current) return;
    started.current = true;
    void changeModel(null, {
      name: 'qwen3.6',
      provider: 'llamacpp',
      alias: 'Qwen3.6',
      subtext: 'Llama Server',
      context_limit: 131072,
    });
  }, [changeModel]);
  return null;
}

createRoot(document.getElementById('root')!).render(
  <ModelAndProviderProvider>
    <Open />
  </ModelAndProviderProvider>
);
`;

const STATUS = {
  sidecar: {
    state: 'stopped',
    model: null,
    hf_spec: null,
    ollama_name: null,
    model_path: null,
    model_source: null,
    port: 11543,
    binary_path: '/opt/homebrew/bin/llama-server',
    build: 'test',
    detail: null,
    context_size: null,
    warmed: false,
  },
  catalog: [
    {
      name: 'qwen3.6',
      display_name: 'Qwen3.6 35B',
      ollama_name: 'qwen3.6:latest',
      hf_spec: 'unsloth/Qwen3.6-35B-A3B-GGUF:UD-Q4_K_M',
      download_size: '23 GB',
      description: 'Large Qwen3.6 MoE model from Ollama library.',
      min_gpu_memory_gib: 48,
      recommended_gpu_memory_gib: 64,
      context_limit: 262144,
      downloaded: false,
      download_status: 'not_downloaded',
      download_source: 'none',
      fallback_downloaded: false,
      fallback_download_status: 'not_downloaded',
      model_path: '/Users/test/.ollama/models/blobs/sha256-0123456789abcdef0123456789abcdef',
      suitable: false,
      suitability_status: 'above_recommendation',
      suitability_message:
        'This model recommends 64 GiB of GPU-addressable memory; this machine reports 8 GiB vram.',
      is_default: true,
    },
  ],
  system: {
    os: 'windows',
    total_memory_gib: 32,
    accelerator_memory_gib: 8,
    accelerator_memory_kind: 'vram',
    default_context_size: 32768,
    model_cache_dir: '/Users/test/.ollama/models',
    model_cache_layout: 'Ollama manifests/blobs; Hugging Face fallback cache under the same root',
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

type Box = { x: number; y: number; width: number; height: number; right: number; bottom: number };

const MODES = ['light', 'dark'] as const;
type Mode = (typeof MODES)[number];

describe('the local-model warm-up dialog in Chromium (V3-R7-1)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the warm-up dialog geometry tests.');
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
      plugins: [stubs],
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

  /** Mount the provider, open the sheet the way the picker does, and let its motion settle. */
  async function open(mode: Mode, size: { width: number; height: number }): Promise<Page> {
    const tab = await browser!.newPage({ viewport: size });
    // Served from an origin, not `setContent`'s about:blank: storage throws on an opaque origin.
    await tab.route('http://warmup.test/', (route) =>
      route.fulfill({
        contentType: 'text/html',
        body:
          `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}" style="--app-font-scale: 1">` +
          `<head><style>${css}</style></head>` +
          `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`,
      })
    );
    await tab.goto('http://warmup.test/');
    await tab.addScriptTag({
      content: `window.electron = { getUserActionKey: async () => 'k', on() { return () => {}; } };
        window.__status = ${JSON.stringify(STATUS)};`,
    });
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[role="dialog"]');
    await tab.waitForTimeout(450);
    return tab;
  }

  const shot = async (tab: Page, name: string) => {
    if (SHOTS) await tab.screenshot({ path: resolve(SHOTS, `${name}.png`) });
  };

  const measure = (tab: Page) =>
    tab.evaluate(() => {
      const box = (el: Element): Box => {
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
      const colour = (value: string) => {
        const probe = document.createElement('div');
        probe.style.backgroundColor = value;
        document.body.append(probe);
        const out = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return out;
      };
      const dialog = document.querySelector('[role="dialog"]')!;
      const scroller = dialog.querySelector('.overflow-y-auto') as HTMLElement;
      const panel = dialog.querySelector('.biorouter-modal-panel') as HTMLElement;
      const header = dialog.firstElementChild!;
      const buttons = [...dialog.querySelectorAll('button')]
        .filter((b) => (b.textContent ?? '').trim() !== 'Close')
        .map((b) => ({ name: (b.textContent ?? '').trim(), box: box(b) }));
      const spinner = dialog.querySelector('.br-spinner');
      return {
        dialog: box(dialog),
        viewport: { width: window.innerWidth, height: window.innerHeight },
        headerRule: parseFloat(getComputedStyle(header).borderBottomWidth),
        scroller: {
          clientHeight: scroller.clientHeight,
          scrollHeight: scroller.scrollHeight,
          overflowY: getComputedStyle(scroller).overflowY,
          box: box(scroller),
        },
        panel: {
          background: getComputedStyle(panel).backgroundColor,
          borderTopWidth: getComputedStyle(panel).borderTopWidth,
          borderTopColor: getComputedStyle(panel).borderTopColor,
          boxShadow: getComputedStyle(panel).boxShadow,
        },
        fill: colour('var(--background-fill)'),
        ground: colour('var(--background-default)'),
        heading: box(dialog.querySelector('h2')!),
        buttons,
        hasClose: !!Array.from(dialog.querySelectorAll('button')).find(
          (b) => (b.textContent ?? '').trim() === 'Close'
        ),
        spinner: spinner ? getComputedStyle(spinner).animationName : null,
      };
    });

  for (const mode of MODES) {
    it(`sits at the shell's top edge, 480 wide, with its detail on the fill (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode, { width: 1440, height: 900 });
      const m = await measure(tab);
      await shot(tab, `warmup-${mode}-1440`);
      await tab.close();
      // ModalShell's top anchor is max(10vh, 48px) = 90 in a 900px window; size md is 480.
      expect(m.dialog.y).toBeCloseTo(90, 0);
      expect(m.dialog.width).toBeCloseTo(480, 0);
      expect(m.headerRule).toBe(1);
      // The detail is the fill recipe: the fill token, a transparent 1px border, no shadow.
      expect(m.panel.background).toBe(m.fill);
      expect(m.panel.background).not.toBe(m.ground);
      expect(m.panel.borderTopWidth).toBe('1px');
      expect(m.panel.borderTopColor).toBe('rgba(0, 0, 0, 0)');
      expect(m.panel.boxShadow).toBe('none');
      // Both buttons on the 32px rung, at the trailing edge on the 16px gutter.
      expect(m.buttons.map((b) => b.name)).toEqual(['Keep previous model', 'Warm up model']);
      for (const { box } of m.buttons) expect(box.height).toBeCloseTo(32, 0);
      expect(m.dialog.right - 1 - m.buttons[1].box.right).toBeCloseTo(16, 0);
    });

    it(`keeps both buttons in view in a short window and scrolls the detail (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode, { width: 1100, height: 430 });
      const m = await measure(tab);
      await shot(tab, `warmup-${mode}-short`);
      await tab.close();
      // The sheet fits the window, with its own margin below.
      expect(m.dialog.y).toBeGreaterThanOrEqual(0);
      expect(m.dialog.bottom).toBeLessThanOrEqual(m.viewport.height - 8);
      // Both buttons are inside the window, not off its bottom edge.
      for (const { box } of m.buttons) {
        expect(box.y).toBeGreaterThanOrEqual(0);
        expect(box.bottom).toBeLessThanOrEqual(m.viewport.height);
      }
      // It is the detail that overflows: the one scroller is shorter than its content, the
      // title is above it and the buttons are below it.
      expect(m.scroller.overflowY).toBe('auto');
      expect(m.scroller.scrollHeight).toBeGreaterThan(m.scroller.clientHeight + 40);
      expect(m.heading.bottom).toBeLessThanOrEqual(m.scroller.box.y);
      for (const { box } of m.buttons) expect(box.y).toBeGreaterThanOrEqual(m.scroller.box.bottom);
    });

    it(`while it warms up offers no way out, and the one spinner turns (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const warm = await open(mode, { width: 1440, height: 900 });
      await warm.evaluate(() => ((window as { __hang?: boolean }).__hang = true));
      await warm.getByRole('button', { name: 'Warm up model' }).click();
      await warm.getByRole('button', { name: 'Warming up…' }).waitFor();
      await warm.waitForTimeout(200);
      const m = await measure(warm);
      await shot(warm, `warmup-${mode}-warming`);
      await warm.close();
      expect(m.hasClose).toBe(false);
      expect(m.buttons.map((b) => b.name)).toEqual(['Keep previous model', 'Warming up…']);
      expect(m.spinner).toBe('br-spin');
    });
  }
});
