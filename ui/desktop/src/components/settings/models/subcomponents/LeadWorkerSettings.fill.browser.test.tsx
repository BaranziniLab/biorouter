// @vitest-environment node
/**
 * The Lead/worker dialog with its mode off, painted by a real engine (vision round 3, V4-R3-3).
 *
 * `.br-model-fields[data-enabled='false'] { opacity: 0.5 }` wrapped controls that already dim
 * themselves: the Select trigger's `opacity-50` and the Input's `disabled:opacity-50`. A field was
 * painted at 0.5 x 0.5 = 0.25, so "Select lead model..." was barely legible and, in dark, a number
 * field's fill faded into the dialog's ground. jsdom runs no CSS and so cannot see a product of
 * two opacities; this test reads the cascade in Chromium:
 *
 * - every field (the two model selects, the three number inputs) is painted at exactly one 50%
 *   step, counted from the control up to the dialog, light and dark;
 * - the labels and their InfoTips stay at full ink, so what a field is for can still be read;
 * - turning the mode on takes the dim away, so the dim is the disabled state's and nothing else's.
 *
 * The real `LeadWorkerSettings` is bundled with esbuild (its config and model contexts are
 * stubbed; the models come from `BIOROUTER_PREDEFINED_MODELS`, which the dialog already reads),
 * `styles/main.css` is compiled by Tailwind with the classes the bundle uses, and
 * `model-dialogs.css` is appended as authored. The instrument is shown to see what it claims to: a
 * case puts the old group rule back and expects 0.25.
 *
 * Set `WS_SCREENSHOT_DIR` to also save a PNG of the dialog in light and dark.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../../../package.json'));
const DIALOGS_CSS = readFileSync(resolve(here, 'model-dialogs.css'), 'utf8');

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { LeadWorkerSettings } from './LeadWorkerSettings';

createRoot(document.getElementById('root')!).render(
  <LeadWorkerSettings isOpen onClose={() => {}} />
);
`;

/** What the dialog imports that is not its business: the config and the model contexts. */
const STUBS: Record<string, string> = {
  // Stable references, like the real context's: the dialog's load effect depends on them, so
  // functions made afresh on every render would restart it forever.
  ConfigContext: `
    const api = {
      read: async (key) => {
        if (key === 'BIOROUTER_MODEL') return 'gpt-4o';
        if (key === 'BIOROUTER_PROVIDER') return 'openai';
        // The dialog reads its mode off the saved lead model, so "on" is a saved one.
        if (key === 'BIOROUTER_LEAD_MODEL' && window.__leadSaved) return 'claude-sonnet';
        if (key === 'BIOROUTER_LEAD_PROVIDER' && window.__leadSaved) return 'anthropic';
        return null;
      },
      upsert: async () => {},
      remove: async () => {},
      getProviders: async () => [],
      getProviderModels: async () => [],
    };
    export const useConfig = () => api;
  `,
  ModelAndProviderContext: `export const useModelAndProvider = () => ({ currentModel: null });`,
};

const stubPlugin: Plugin = {
  name: 'lead-worker-stubs',
  setup(b) {
    b.onResolve({ filter: /(\/ConfigContext|\/ModelAndProviderContext)$/ }, (args) => {
      const name = Object.keys(STUBS).find((key) => args.path.endsWith(key));
      return name ? { path: name, namespace: 'stub' } : undefined;
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

/** What the dialog's controls and words are painted at, counted up to (not through) the dialog. */
type Paint = {
  selects: number[];
  numbers: number[];
  labels: number[];
  tips: number[];
  groupOpacity: string;
  enabled: string | null;
};

const MODEL_IDS = ['lead-model', 'worker-model'];
const NUMBER_IDS = ['lead-turns', 'failure-threshold', 'fallback-turns'];

describe('the Lead/worker dialog with its mode off (V4-R3-3)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Lead/worker fill tests.');
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

  /** The dialog as the app mounts it, with the mode off (nothing saved) or on (`enabled`). */
  async function open(
    mode: 'light' | 'dark',
    options: { enabled?: boolean; sheet?: string } = {}
  ): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 900 } });
    const sheet = options.sheet ?? css + DIALOGS_CSS;
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${sheet}</style></head>` +
        `<body style="margin:0" class="bg-background-canvas"><div id="root"></div></body></html>`
    );
    await tab.evaluate((enabled) => {
      const w = window as unknown as { appConfig: unknown };
      w.appConfig = {
        get: (key: string) =>
          key === 'BIOROUTER_PREDEFINED_MODELS'
            ? JSON.stringify([
                { name: 'gpt-4o', alias: 'GPT-4o', provider: 'openai' },
                { name: 'claude-sonnet', alias: 'Claude Sonnet', provider: 'anthropic' },
              ])
            : undefined,
      };
      (window as unknown as { __leadSaved: boolean }).__leadSaved = enabled;
    }, options.enabled === true);
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#lead-turns');
    // The dialog's own entrance animates opacity; the measure stops at the dialog, but wait it out
    // so a screenshot is of the settled panel.
    await tab.waitForTimeout(400);
    return tab;
  }

  const measure = (tab: Page): Promise<Paint> =>
    tab.evaluate(
      ({ modelIds, numberIds }) => {
        // The product of `opacity` from the element up to, and not including, the dialog box.
        const painted = (el: Element | null): number => {
          let product = 1;
          for (let n: Element | null = el; n && n.getAttribute('role') !== 'dialog'; ) {
            product *= parseFloat(getComputedStyle(n).opacity);
            n = n.parentElement;
          }
          return Math.round(product * 1000) / 1000;
        };
        const byId = (id: string) => document.getElementById(id);
        const fields = document.querySelector('.br-model-fields') as HTMLElement;
        return {
          selects: modelIds.map((id) => painted(byId(id))),
          numbers: numberIds.map((id) => painted(byId(id))),
          labels: [...modelIds, ...numberIds].map((id) =>
            painted(document.querySelector(`label[for="${id}"]`))
          ),
          tips: [...document.querySelectorAll('.br-model-form-head button')].map((b) => painted(b)),
          groupOpacity: getComputedStyle(fields).opacity,
          enabled: fields.getAttribute('data-enabled'),
        };
      },
      { modelIds: MODEL_IDS, numberIds: NUMBER_IDS }
    );

  /**
   * How far each piece of text in a field stands off its own ground once the field is painted,
   * as a WCAG contrast ratio. A dimmed field is one layer composited over the dialog at the
   * product of its opacities, so the ink and the fill are each blended with the dialog's ground
   * by that product, then compared.
   */
  const contrast = (
    tab: Page
  ): Promise<{ placeholder: number; selected: number; number: number }> =>
    tab.evaluate(() => {
      type Rgb = [number, number, number];
      const parse = (css: string): { rgb: Rgb; a: number } => {
        const m = css.match(/[\d.]+/g)!.map(Number);
        if (css.startsWith('color(')) {
          return { rgb: [m[0] * 255, m[1] * 255, m[2] * 255], a: m.length > 3 ? m[3] : 1 };
        }
        return { rgb: [m[0], m[1], m[2]], a: m.length > 3 ? m[3] : 1 };
      };
      const mix = (top: Rgb, over: Rgb, p: number): Rgb =>
        [0, 1, 2].map((i) => top[i] * p + over[i] * (1 - p)) as Rgb;
      const lum = (c: Rgb) => {
        const [r, g, b] = c.map((v) => {
          const x = v / 255;
          return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
        });
        return 0.2126 * r + 0.7152 * g + 0.0722 * b;
      };
      const ratio = (a: Rgb, b: Rgb) => {
        const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
        return (hi + 0.05) / (lo + 0.05);
      };
      const dialog = document.querySelector('[role="dialog"]') as HTMLElement;
      const ground = parse(getComputedStyle(dialog).backgroundColor).rgb;
      const painted = (el: Element | null): number => {
        let product = 1;
        for (let n: Element | null = el; n && n.getAttribute('role') !== 'dialog'; ) {
          product *= parseFloat(getComputedStyle(n).opacity);
          n = n.parentElement;
        }
        return product;
      };
      // The fill under the text: the field's own (or its control's) background over the ground.
      const standOff = (text: Element, field: Element): number => {
        const p = painted(field);
        const ink = parse(getComputedStyle(text).color);
        const fillCss = parse(getComputedStyle(field).backgroundColor);
        const fill = mix(fillCss.rgb, ground, fillCss.a);
        // Inside the layer the ink is on the fill; the layer then goes over the ground at p.
        const inkSeen = mix(mix(ink.rgb, fill, ink.a), ground, p);
        const fillSeen = mix(fill, ground, p);
        return ratio(inkSeen, fillSeen);
      };
      const control = (id: string) =>
        document.getElementById(id)!.closest('[class*="h-control-md"]') as HTMLElement;
      const leadControl = control('lead-model');
      const workerControl = control('worker-model');
      const placeholder = leadControl.querySelector('[id$="-placeholder"]')!;
      const selected = [...workerControl.querySelectorAll('span')].find(
        (el) => el.textContent === 'GPT-4o'
      )!;
      const number = document.getElementById('lead-turns')!;
      return {
        placeholder: standOff(placeholder, leadControl),
        selected: standOff(selected, workerControl),
        number: standOff(number, number),
      };
    });

  const shoot = async (tab: Page, name: string) => {
    const dir = process.env.WS_SCREENSHOT_DIR;
    if (!dir) return;
    mkdirSync(dir, { recursive: true });
    await tab.screenshot({ path: resolve(dir, `${name}.png`) });
  };

  it('paints each field at one 50% step, not two, light and dark', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const paint = await measure(tab);
        expect(paint.enabled, `${mode} mode is off`).toBe('false');
        expect(paint.groupOpacity, `${mode} the group itself does not dim`).toBe('1');
        expect(paint.selects, `${mode} model selects`).toEqual([0.5, 0.5]);
        expect(paint.numbers, `${mode} number fields`).toEqual([0.5, 0.5, 0.5]);
        await shoot(tab, `lead-worker-off-${mode}`);
      } finally {
        await tab.close();
      }
    }
  });

  it('keeps the words in the fields readable while they are off', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const c = await contrast(tab);
        // One 50% step is what every disabled control takes, and it leaves a chosen model and a
        // count above 2.5:1 (measured 2.95 light, 4.5 dark). The old double step left dark at 2.1.
        expect(c.selected, `${mode} chosen model`).toBeGreaterThanOrEqual(2.5);
        expect(c.number, `${mode} count`).toBeGreaterThanOrEqual(2.5);
        // The placeholder is muted by design and stays above 2:1 (2.1 light, 2.7 dark). At the old
        // double step it was 1.6:1 in dark.
        expect(c.placeholder, `${mode} placeholder`).toBeGreaterThanOrEqual(2);
      } finally {
        await tab.close();
      }
    }
  });

  it('keeps every label and InfoTip at full ink while the fields are off', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const paint = await measure(tab);
        expect(paint.labels, `${mode} labels`).toEqual([1, 1, 1, 1, 1]);
        expect(paint.tips.length, `${mode} tips found`).toBeGreaterThanOrEqual(5);
        expect(
          paint.tips.every((p) => p === 1),
          `${mode} tips ${paint.tips.join(',')}`
        ).toBe(true);
      } finally {
        await tab.close();
      }
    }
  });

  it('takes the dim away when the mode is on, so the dim is the disabled state alone', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light', { enabled: true });
    try {
      const paint = await measure(tab);
      expect(paint.enabled).toBe('true');
      expect(paint.selects).toEqual([1, 1]);
      expect(paint.numbers).toEqual([1, 1, 1]);
      await shoot(tab, 'lead-worker-on-light');
    } finally {
      await tab.close();
    }
  });

  it('sees the old defect: the group rule back makes a field 0.25', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('dark', {
      sheet: css + DIALOGS_CSS + ".br-model-fields[data-enabled='false'] { opacity: 0.5; }",
    });
    try {
      const paint = await measure(tab);
      expect(paint.selects).toEqual([0.25, 0.25]);
      expect(paint.numbers).toEqual([0.25, 0.25, 0.25]);
      // Labels sit inside the group too, so the old rule dimmed them as well.
      expect(paint.labels.slice(0, 2)).toEqual([0.5, 0.5]);
      // And the words fall under the floors the test above holds.
      const c = await contrast(tab);
      expect(c.selected).toBeLessThan(2.5);
      expect(c.placeholder).toBeLessThan(2);
    } finally {
      await tab.close();
    }
  });
});
