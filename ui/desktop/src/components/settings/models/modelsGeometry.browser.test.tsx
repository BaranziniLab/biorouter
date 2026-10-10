// @vitest-environment node
/**
 * The Models tab's rows, measured in a real layout engine (codex simplicity redesign, spec 3.13
 * and 5.7).
 *
 * jsdom lays nothing out and runs no CSS, so a component test can only say "this class is on this
 * node". The acceptance numbers for the tab are geometry: every Default model and Danger zone row
 * is one 40px line, a local model is two lines and 56px (a name over one facts line), the Local
 * models header stays the 24px every section header is (the 28px refresh button reaches into the
 * row padding instead of growing it), the Install buttons share one right edge, and the hover
 * `⋯` sits on the same right edge as the Change… button above, hidden until the row is pointed
 * at, without moving anything beside it.
 *
 * The real `ModelsSection` is bundled with esbuild (the daemon, config, toast and Ollama
 * dependencies stubbed), rendered by React in Chromium, and styled by the authored
 * `@ws WS-PRIMITIVES` blocks of `styles/main.css` plus `models.css`, with the token values read
 * from that file. Tailwind is not run, so only the utilities the measured boxes depend on are
 * written out below.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../../styles/main.css'), 'utf8');
const MODELS_CSS = readFileSync(resolve(here, 'models.css'), 'utf8');

const TOKEN_NAMES = [
  '--control-md',
  '--control-sm',
  '--radius-full',
  '--radius-element',
  '--radius-inner',
  '--dur-fast-min',
  '--dur-fast',
  '--dur-fast-max',
  '--ease-out',
  '--ease-spring',
  '--row-height',
  '--radius-md',
  '--font-mono',
];

/** The first declaration of each token in main.css, so a token change reaches this test. */
function readTokens(): string {
  const lines = TOKEN_NAMES.map((name) => {
    const match = MAIN_CSS.match(new RegExp(`\\n\\s*${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
    if (!match) throw new Error(`main.css does not declare ${name}`);
    return `  ${name}: ${match[1].trim()};`;
  });
  return `:root {\n${lines.join('\n')}\n}`;
}

/** Every `@ws WS-PRIMITIVES` block of main.css, in file order. */
function primitiveBlocks(): string {
  const out: string[] = [];
  const re =
    /\/\* @ws WS-PRIMITIVES begin: ([^*]+?) \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: \1 \*\//g;
  for (const match of MAIN_CSS.matchAll(re)) out.push(match[2]);
  if (out.length === 0) throw new Error('main.css has no @ws WS-PRIMITIVES block');
  return out.join('\n');
}

/** Tailwind's preflight, the roles and the few utilities the measured boxes use. */
const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
button { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
h2, h3, p { margin: 0; font-size: inherit; font-weight: inherit; }
svg { display: block; }
button svg { width: 16px; height: 16px; flex: none; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
.text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
.text-secondary { font-size: 13px; line-height: 18px; }
.text-supporting, .text-caps { font-size: 12px; line-height: 16px; }
.tabular-nums { font-variant-numeric: tabular-nums; }
.flex { display: flex; } .inline-flex { display: inline-flex; } .items-center { align-items: center; }
.items-start { align-items: flex-start; } .justify-center { justify-content: center; }
.flex-1 { flex: 1 1 0%; } .min-w-0 { min-width: 0; } .shrink-0 { flex-shrink: 0; }
.gap-1\\.5 { gap: 6px; } .gap-2 { gap: 8px; } .whitespace-nowrap { white-space: nowrap; }
.px-3 { padding-inline: 12px; } .p-0 { padding: 0; } .mt-3 { margin-top: 12px; }
.rounded-element { border-radius: 8px; }
.h-control-sm { height: var(--control-sm); } .w-control-sm { width: var(--control-sm); }
.h-4 { height: 16px; } .h-5 { height: 20px; } .h-3 { height: 12px; } .w-40 { width: 160px; }
.w-56 { width: 224px; } .biorouter-settings-list { display: flex; flex-direction: column; }
.biorouter-settings-section-header { margin-bottom: 8px; }
.biorouter-settings-section { padding-block: 14px; }
.br-switch { flex: none; }
:root { --control-track-off: #ebebe8; --background-accent: #b85a32; --border-control: #86847f;
  --background-default: #fff; --text-on-accent: #fff; --text-muted: #666; --text-default: #111;
  --text-subtle: #777; --text-danger: #b00020; --background-muted: #f2f2f0;
  --border-focus: #555; --background-focus: #ddd; --control-sm: 28px;
  --shadow-raised: 0 1px 2px rgb(0 0 0 / 0.1); --inset-hairline: inset 0 0 0 1px #ddd;
  --overlay-hover: rgb(0 0 0 / 0.05); --border-subtle: #ccc; }
`;

/** What `import` specifiers the real components reach for that a page cannot serve. */
const STATUS = {
  sidecar: { state: 'stopped', warmed: false, build: 'b1', detail: null, model: null },
  catalog: [
    ['gemma4', 'Gemma 4 E4B', true, 'suitable', 'Fast, ~4B active parameters'],
    ['gemma4-12b', 'Gemma 4 12B', false, 'above_recommendation', 'Fast, dense 12B, quick to load'],
    ['gemma4-31b', 'Gemma 4 31B', false, 'suitable', 'Moderate, dense 31B'],
  ].map(([name, display, installed, fit, speed]) => ({
    name,
    display_name: display,
    family: 'Gemma 4',
    ollama_name: `${name}:latest`,
    official_url: 'https://ollama.com',
    hf_spec: `google/${name}:Q4_0`,
    download_size: '9.6 GB',
    description: 'A model',
    min_gpu_memory_gib: 16,
    recommended_gpu_memory_gib: 16,
    context_limit: 131072,
    active_params_b: 4,
    speed_hint: speed,
    is_default: false,
    downloaded: installed,
    download_status: installed ? 'downloaded' : 'not_downloaded',
    download_source: installed ? 'ollama' : 'none',
    fallback_downloaded: false,
    fallback_download_status: 'not_downloaded',
    model_path: null,
    suitable: fit === 'suitable',
    suitability_status: fit,
    suitability_message: 'Recommended.',
  })),
  system: {
    os: 'macos',
    total_memory_gib: 64,
    accelerator_memory_gib: 48,
    accelerator_memory_kind: 'apple_unified',
    default_context_size: 131072,
    model_cache_dir: '/models',
    model_cache_layout: 'ollama',
  },
};

const STUBS: Array<{ filter: RegExp; contents: string }> = [
  {
    filter: /\/ConfigContext$/,
    contents: `
      const api = { read: async () => '', remove: async () => {}, getProviders: async () => [] };
      export const useConfig = () => api;
    `,
  },
  {
    filter: /\/ModelAndProviderContext$/,
    contents: `
      const value = {
        getCurrentModelDisplayName: async () => 'gpt-5.6-sol',
        getCurrentProviderDisplayName: async () => 'Versa Azure',
        currentModel: 'gpt-5.6-sol',
        currentProvider: 'versa_azure',
      };
      export const UNKNOWN_PROVIDER_MSG = 'unknown';
      export const UNKNOWN_PROVIDER_TITLE = 'unknown';
      export const useModelAndProvider = () => value;
    `,
  },
  { filter: /\/SwitchModelModal$/, contents: `export const SwitchModelModal = () => null;` },
  {
    filter: /\/api$/,
    contents: `
      const status = ${JSON.stringify(STATUS)};
      export const llamacppStatus = async () => ({ data: status });
      export const llamacppEnsure = async () => ({ data: status });
      export const llamacppWarmup = async () => ({ data: { output: 'ok', sidecar: status.sidecar } });
      export const llamacppDelete = async () => ({ data: { deleted_fallback_cache: false, status } });
    `,
  },
  {
    filter: /\/toasts$/,
    contents: `
      export const toastError = () => {};
      export const toastService = { error() {}, success() {} };
    `,
  },
  {
    filter: /\/ollamaDetection$/,
    contents: `
      export const checkOllamaStatus = async () => ({ isRunning: false });
      export const deleteOllamaModel = async () => false;
      export const pullOllamaModel = async () => false;
    `,
  },
];

const stubPlugin: Plugin = {
  name: 'stubs',
  setup(b) {
    for (const { filter, contents } of STUBS) {
      b.onResolve({ filter }, (args) =>
        // Only the specifiers the components under test import; the stub files do not exist.
        args.importer.includes('/settings/') || args.importer.includes('/privacy/')
          ? { path: args.path, namespace: 'stub', pluginData: contents }
          : undefined
      );
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      loader: 'js',
    }));
    // The stylesheets are added to the page by hand.
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import ModelsSection from './ModelsSection';

createRoot(document.getElementById('root')!).render(
  <div style={{ width: 712, padding: 16 }}>
    <ModelsSection setView={() => {}} />
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

const EPSILON = 0.5;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

describe('the Models tab rows in a real layout engine (spec 3.13, 5.7)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the row geometry.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [stubPlugin],
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css = PREFLIGHT + readTokens() + primitiveBlocks() + MODELS_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 1000 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    // The inventory loads asynchronously: wait for a model row and the model's value.
    await tab.waitForSelector('[data-testid^="local-model-"]');
    await tab.waitForSelector('.br-setting-row-value');
    await tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );
    return tab;
  }

  const rowsOf = (tab: Page, section: string) =>
    tab.evaluate((id) => {
      return [...document.querySelectorAll(`#${id} .biorouter-settings-row`)].map((row) => ({
        label: row.querySelector('.br-setting-row-label')?.textContent?.trim() ?? '',
        height: row.getBoundingClientRect().height,
      }));
    }, section);

  it('draws the Model, Providers and Reset rows as one 40px line each', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const model = await rowsOf(tab, 'settings-default-model');
      expect(model.map((row) => row.label)).toEqual(['Model', 'Providers']);
      for (const row of model) near(row.height, 40, `${row.label} row`);
      const danger = await rowsOf(tab, 'settings-models-danger-zone');
      expect(danger.map((row) => row.label)).toEqual(['Reset provider and model']);
      near(danger[0].height, 40, 'Reset row');
    } finally {
      await tab.close();
    }
  });

  it('draws a local model as a name over one facts line, 56px, and nothing taller', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() =>
        [...document.querySelectorAll('[data-testid^="local-model-"]')].map((row) => {
          const rect = (el: Element | null) => el!.getBoundingClientRect();
          const name = row.querySelector('.br-setting-row-label')!;
          const facts = name.closest('.br-setting-row-text')!.lastElementChild!;
          return {
            height: rect(row).height,
            nameBottom: rect(name).bottom,
            factsTop: rect(facts).top,
            factsHeight: rect(facts).height,
          };
        })
      );
      expect(m).toHaveLength(3);
      for (const row of m) {
        near(row.height, 56, 'model row');
        // The facts line is one 16px line directly under the name's 20px line.
        near(row.factsHeight, 16, 'facts line');
        near(row.factsTop, row.nameBottom, 'facts under the name');
      }
    } finally {
      await tab.close();
    }
  });

  it('keeps the Local models header at 24px with the count and refresh at its end', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const h = await tab.evaluate(() => {
        const section = document.getElementById('settings-local-models')!;
        const rect = (el: Element | null) => el!.getBoundingClientRect();
        const header = section.querySelector('.br-setting-section-header')!;
        const rowRight = rect(section.querySelector('.biorouter-settings-row')).right;
        return {
          header: rect(header).height,
          refresh: rect(header.querySelector('.br-local-models-refresh')),
          count: rect(header.querySelector('.tabular-nums')),
          headerRight: rect(header).right,
          rowRight,
        };
      });
      // The 28px refresh button reaches into the row padding rather than growing the header.
      near(h.header, 24, 'Local models header');
      near(h.refresh.width, 28, 'refresh button');
      expect(h.count.right).toBeLessThanOrEqual(h.refresh.left + EPSILON);
      // It ends 12px in from the list, with every row's own content.
      near(h.refresh.right, h.rowRight - 12, 'refresh against the row content edge');
    } finally {
      await tab.close();
    }
  });

  it('lines the Install buttons and the hover ⋯ up on the right edge, and moves nothing on hover', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const measure = () =>
        tab.evaluate(() => {
          const rect = (el: Element | null) => el!.getBoundingClientRect().toJSON();
          const rows = [...document.querySelectorAll('[data-testid^="local-model-"]')];
          const change = document.querySelector(
            '#settings-default-model button[aria-label="Switch models"]'
          )!;
          return {
            change: rect(change),
            installs: rows
              .map((row) => row.querySelector('button[aria-label^="Install "]'))
              .filter(Boolean)
              .map((button) => rect(button)),
            menus: rows.map((row) => {
              const button = row.querySelector('button[aria-label^="More actions"]')!;
              const meta = row.querySelector('.br-row-actions-meta');
              return {
                box: rect(button),
                opacity: Number(getComputedStyle(button.closest('.br-row-actions')!).opacity),
                // An installed row's quiet status; the ⋯ takes its cell over on hover.
                meta: meta
                  ? { box: rect(meta), opacity: Number(getComputedStyle(meta).opacity) }
                  : null,
              };
            }),
          };
        });

      const before = await measure();
      // Two of the three models are not installed.
      expect(before.installs).toHaveLength(2);
      near(before.installs[0].right, before.installs[1].right, 'Install buttons share an edge');
      // The ⋯ is on the Change… button's right edge, and hidden at rest.
      for (const menu of before.menus) {
        near(menu.box.right, before.change.right, 'the ⋯ against the Change… button');
        expect(menu.opacity).toBe(0);
      }

      // Only the installed model says "Installed", on the same right edge as the ⋯ and the buttons.
      expect(before.menus[0].meta).not.toBeNull();
      expect(before.menus[1].meta).toBeNull();
      near(before.menus[0].meta!.box.right, before.change.right, 'Installed against Change…');
      expect(before.menus[0].meta!.opacity).toBe(1);

      await tab.locator('[data-testid="local-model-gemma4"]').hover();
      await tab.waitForTimeout(250);
      const installedHover = await measure();
      expect(installedHover.menus[0].meta!.opacity).toBe(0);
      expect(installedHover.menus[0].opacity).toBe(1);

      await tab.locator('[data-testid="local-model-gemma4-12b"]').hover();
      await tab.waitForTimeout(250);
      const after = await measure();
      expect(after.menus[1].opacity).toBe(1);
      expect(after.menus[0].opacity).toBe(0);
      // Revealing the ⋯ moves no box: opacity only.
      near(after.installs[0].left, before.installs[0].left, 'Install button after hover');
      near(after.menus[1].box.left, before.menus[1].box.left, '⋯ after hover');
    } finally {
      await tab.close();
    }
  });
});
