// @vitest-environment node
/**
 * The Llama Server panel's primary appears at rest (vision FR-R3-1).
 *
 * Switching to Local, or reopening Llama Server, mounted "Download and start (7.6 GB)" disabled for
 * one commit and then enabled it, so the button's 125 ms opacity transition faded it in from half
 * on every mount (the reviewer measured 0.50 at 56 ms, 0.85 at 81 ms, 1.0 at 183 ms). The selected
 * model started as '' and was set from the status in an effect. Principle 9: a value that loads
 * asynchronously does not animate. The panel now reads the model straight from the status until a
 * person picks one, so the primary is enabled in the frame it first appears.
 *
 * jsdom runs no transitions and no frames, so the real card is bundled with esbuild (API, toasts and
 * config stubbed), `main.css` is compiled by Tailwind, and Chromium watches the button: a
 * MutationObserver records any change of its `disabled` attribute, and every animation frame
 * samples its computed opacity, from before the mount until well past the transition.
 */
import { readFileSync } from 'node:fs';
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

/** The card, and a switch that unmounts and remounts it as a tab switch does. */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import LlamaServerInlineCard from './LlamaServerInlineCard';

function Panel() {
  const [shown, setShown] = React.useState(true);
  (window as unknown as { __setShown: (v: boolean) => void }).__setShown = setShown;
  return (
    <div style={{ maxWidth: 560, margin: '48px auto' }}>
      {shown ? <LlamaServerInlineCard onSuccess={() => {}} /> : <p>Other tab</p>}
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<Panel />);
`;

const model = (name: string, display: string, size: string) => `{
  name: '${name}', display_name: '${display}', family: 'Gemma 4', ollama_name: '${name}:latest',
  official_url: 'https://ollama.com/library/${name}', hf_spec: 'google/${name}:Q4_0',
  download_size: '${size}', description: 'A model', min_gpu_memory_gib: 16,
  recommended_gpu_memory_gib: 16, context_limit: 262144, active_params_b: 4,
  speed_hint: 'Fast', is_default: ${name === 'gemma4-12b'}, downloaded: false,
  download_status: 'not_downloaded', download_source: 'none', fallback_downloaded: false,
  fallback_download_status: 'not_downloaded', model_path: null, suitable: true,
  suitability_status: 'suitable', suitability_message: 'Recommended for this machine.'
}`;

const STUBS: Record<string, string> = {
  api: `
    export const llamacppStatus = async () => {
      await new Promise((r) => setTimeout(r, 60));
      return {
        data: {
          sidecar: { state: 'stopped', warmed: false, build: 'test', detail: null, model: null },
          catalog: [${model('gemma4', 'Gemma 4 E4B', '9.6 GB')}, ${model('gemma4-12b', 'Gemma 4 12B', '7.6 GB')}],
          system: { os: 'macos', total_memory_gib: 64, accelerator_memory_gib: 64,
                    accelerator_memory_kind: 'apple_unified', default_context_size: 131072,
                    model_cache_dir: '/tmp/models', model_cache_layout: 'test' },
        },
      };
    };
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

interface Watch {
  /** Every change of the start button's `disabled` attribute after it appeared. */
  toggles: number;
  /** Whether the button was disabled when it was first seen. */
  firstDisabled: boolean | null;
  /** Computed opacity on every frame the button existed. */
  opacities: number[];
  label: string;
}

/** Arm the watchers, run `act` in the page, then watch for `ms`. */
const WATCH = (ms: number) => `
  new Promise((resolveWatch) => {
    const watch = { toggles: 0, firstDisabled: null, opacities: [], label: '' };
    const seen = new WeakSet();
    const button = () => document.querySelector('[data-testid="llamacpp-start"]');
    const note = () => {
      const b = button();
      if (b && !seen.has(b)) {
        seen.add(b);
        if (watch.firstDisabled === null) watch.firstDisabled = b.disabled;
      }
    };
    const observer = new MutationObserver((records) => {
      for (const record of records) {
        if (record.type === 'attributes' && record.target === button()) watch.toggles += 1;
      }
      note();
    });
    observer.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['disabled'],
    });
    const t0 = performance.now();
    const frame = () => {
      note();
      const b = button();
      if (b) watch.opacities.push(Number(getComputedStyle(b).opacity));
      if (performance.now() - t0 < ${ms}) requestAnimationFrame(frame);
      else {
        observer.disconnect();
        watch.label = button()?.textContent ?? '';
        resolveWatch(watch);
      }
    };
    requestAnimationFrame(frame);
    window.__act?.();
  })
`;

describe('the Llama Server primary appears at rest (FR-R3-1)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Llama Server mount tests.');
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

  async function blank(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 700 } });
    tab.setDefaultTimeout(8000);
    await tab.setContent(
      `<!doctype html><html><head><style>${css + ONBOARDING_CSS}</style></head>` +
        `<body style="margin:0" class="bg-background-canvas"><div id="root"></div></body></html>`
    );
    return tab;
  }

  const expectAtRest = (watch: Watch) => {
    expect(watch.firstDisabled, 'the primary is enabled when it first appears').toBe(false);
    expect(watch.toggles, 'its disabled attribute never changes on the way in').toBe(0);
    expect(watch.opacities.length).toBeGreaterThan(5);
    expect(Math.min(...watch.opacities), 'no fade from half opacity').toBeGreaterThanOrEqual(0.99);
    expect(watch.label).toBe('Download and start (7.6 GB)');
  };

  it('mounts the primary enabled on first load, after the status check', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await blank();
    try {
      // Arm before the card exists: the first mount is the script tag itself.
      const watching = tab.evaluate(WATCH(900));
      await tab.addScriptTag({ content: script });
      expectAtRest((await watching) as Watch);
    } finally {
      await tab.close();
    }
  });

  it('remounts the primary enabled, at full opacity, as a tab switch back does', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await blank();
    try {
      await tab.addScriptTag({ content: script });
      await tab.waitForSelector('[data-testid="llamacpp-start"]:not([disabled])');
      await tab.evaluate(() => {
        (window as unknown as { __setShown: (v: boolean) => void }).__setShown(false);
      });
      await tab.waitForSelector('text=Other tab');
      await tab.evaluate(() => {
        (window as unknown as { __act: () => void }).__act = () =>
          (window as unknown as { __setShown: (v: boolean) => void }).__setShown(true);
      });
      expectAtRest((await tab.evaluate(WATCH(500))) as Watch);
    } finally {
      await tab.close();
    }
  });
});
