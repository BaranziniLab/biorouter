// @vitest-environment node
/**
 * First run and the provider page on the fill system, in a real layout engine (owner message 13,
 * R-27, design.md 9.11; WS-SETTINGS-M's surfaces).
 *
 * The claims are about the cascade, which jsdom does not run: a private ground variable the
 * first-run page sets once and every filled control inside it follows, a box that rests where a
 * field would, a row that paints a fill under the pointer while the button inside it steps back
 * one ground, and a hairline that goes from a row by scope. So the real components are bundled
 * with esbuild, the real `styles/main.css` is compiled by Tailwind with the classes the bundle uses,
 * `providers.css` and `onboarding.css` are appended as authored, and Chromium paints them. Colours
 * are compared with probes that paint the token itself, so a token value can change without this
 * file changing.
 *
 * Set `WS_SCREENSHOT_DIR` to also save a PNG of each ground in light and dark.
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
const ONBOARDING_CSS = readFileSync(resolve(here, '../../onboarding/onboarding.css'), 'utf8');

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ProviderCard } from './subcomponents/ProviderCard';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { SegmentedControl } from '../../ui/segmented-control';
import { CopyField } from '../../ui/copy-field';

const provider = (name: string, display: string, configured: boolean) => ({
  name,
  is_configured: configured,
  provider_type: 'Builtin',
  metadata: {
    name,
    display_name: display,
    description: 'Models through the ' + display + ' API',
    default_model: 'm',
    known_models: [],
    model_doc_link: '',
    config_keys: [],
    tier: 'public',
    runs_locally: false,
    institutions: [],
    allows_unlisted_models: true,
  },
});

function Ground({ id, muted }: { id: string; muted: boolean }) {
  const [flavor, setFlavor] = React.useState('azure');
  return (
    <div
      id={id}
      className={muted ? 'br-first-run-ground' : ''}
      style={{
        background: muted ? 'var(--background-muted)' : 'var(--background-canvas)',
        padding: 24,
        width: 620,
      }}
    >
      <div className="br-provider-catalog">
        <div className="biorouter-settings-list" id={id + '-list'}>
          <ProviderCard provider={provider('openai', 'OpenAI', false) as never} onConfigure={() => {}} onLaunch={() => {}} isOnboarding={false} />
          <ProviderCard provider={provider('anthropic', 'Anthropic', true) as never} onConfigure={() => {}} onLaunch={() => {}} isOnboarding={false} />
        </div>
        <div className="br-provider-panel" style={{ display: 'grid', gap: 12 }}>
          <div id={id + '-box'} className="br-first-run-box br-fill-surface p-3">
            <p className="text-supporting text-text-muted">Gemma 4 · 9.6 GB download</p>
            <Button id={id + '-box-button'} variant="secondary" size="sm">Check again</Button>
            <div className="br-progress" id={id + '-progress'}><div className="br-progress__fill" style={{ width: '40%' }} /></div>
          </div>
          <Input id={id + '-field'} aria-label="API key" placeholder="Paste your API key here…" />
          <select id={id + '-select'} className="br-field block h-control-md w-full rounded-element border px-2 text-label"><option>Gemma 4 E4B</option></select>
          <div><Button id={id + '-button'} variant="secondary" size="sm">Check again</Button></div>
          <div><SegmentedControl aria-label="Gateway" options={[{ value: 'azure', label: 'Azure OpenAI (GPT)' }, { value: 'bedrock', label: 'Bedrock (Claude)' }]} value={flavor} onValueChange={setFlavor} /></div>
          <CopyField value="npm install -g @anthropic-ai/claude-code@latest" label="install command" />
        </div>
      </div>
      <div className="biorouter-settings-list" id={id + '-plain'}>
        <div className="biorouter-settings-row br-setting-row">Plain row</div>
        <div className="biorouter-settings-row br-setting-row">Plain row two</div>
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(
  <>
    <Ground id="canvas" muted={false} />
    <Ground id="muted" muted />
  </>
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

describe('first run and the provider page on the fill system (R-27)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the first-run fill tests.');
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

  async function open(
    mode: 'light' | 'dark',
    options: { sheet?: string; forcedColors?: boolean } = {}
  ): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 700, height: 1500 } });
    if (options.forcedColors) await tab.emulateMedia({ forcedColors: 'active' });
    const sheet = options.sheet ?? css + PROVIDERS_CSS + ONBOARDING_CSS;
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${sheet}</style></head>` +
        `<body style="margin:0"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#muted-copy, #muted-box');
    await tab.waitForTimeout(300);
    return tab;
  }

  /** The computed colour a probe inside `within` paints for `value` (a token, in that place). */
  const paint = (tab: Page, value: string, within = 'body') =>
    tab.evaluate(
      ([v, w]) => {
        const probe = document.createElement('div');
        probe.style.backgroundColor = v;
        document.querySelector(w)!.append(probe);
        const out = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return out;
      },
      [value, within] as const
    );
  const ground = (tab: Page, selector: string) =>
    tab.evaluate((s) => getComputedStyle(document.querySelector(s)!).backgroundColor, selector);

  const shoot = async (tab: Page, name: string) => {
    const dir = process.env.WS_SCREENSHOT_DIR;
    if (!dir) return;
    mkdirSync(dir, { recursive: true });
    await tab.screenshot({ path: resolve(dir, `${name}.png`), fullPage: true });
  };

  it('rests every filled control and box on the fill on the canvas, and one step deeper on the muted first-run ground', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const fill = await paint(tab, 'var(--background-fill)');
        const strong = await paint(tab, 'var(--background-fill-strong)');
        const page = await paint(tab, 'var(--background-default)');
        const muted = await paint(tab, 'var(--background-muted)');
        // The reason for the rule: the rest fill IS the muted ground, so a control would vanish.
        expect(fill, `${mode} the rest fill equals the muted ground`).toBe(muted);
        expect(strong, `${mode} the strong step differs from the muted ground`).not.toBe(muted);

        for (const part of ['field', 'select', 'button', 'box']) {
          expect(await ground(tab, `#canvas-${part}`), `${mode} canvas ${part}`).toBe(fill);
          expect(await ground(tab, `#muted-${part}`), `${mode} muted ${part}`).toBe(strong);
        }
        // A control inside a filled box steps back to the page ground, on both grounds.
        expect(await ground(tab, '#canvas-box-button'), `${mode} canvas box button`).toBe(page);
        expect(await ground(tab, '#muted-box-button'), `${mode} muted box button`).toBe(page);
        // The two shared primitives that paint their own ground take the step on the muted page.
        expect(await ground(tab, '#muted .br-segmented'), `${mode} muted track`).toBe(strong);
        expect(await ground(tab, '#muted .biorouter-copy-field'), `${mode} muted copy`).toBe(
          strong
        );
        await shoot(tab, `first-run-fill-${mode}`);
      } finally {
        await tab.close();
      }
    }
  });

  it('keeps a transparent 1px edge on every box, which forced colours draw', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light', { forcedColors: true });
    try {
      for (const id of ['#canvas-box', '#muted-box']) {
        const edge = await tab.evaluate((s) => {
          const cs = getComputedStyle(document.querySelector(s)!);
          return { width: cs.borderTopWidth, color: cs.borderTopColor };
        }, id);
        expect(edge.width, id).toBe('1px');
        // CanvasText, not transparent: the box keeps its outline there.
        expect(edge.color, id).not.toBe('rgba(0, 0, 0, 0)');
      }
    } finally {
      await tab.close();
    }
    const light = await open('light');
    try {
      const color = await light.evaluate(
        () => getComputedStyle(document.querySelector('#muted-box')!).borderTopColor
      );
      expect(color).toBe('rgba(0, 0, 0, 0)');
    } finally {
      await light.close();
    }
  });

  it('draws no hairline under a catalog row, and keeps the shared one under any other list', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      const after = (selector: string) =>
        tab.evaluate(
          (s) => getComputedStyle(document.querySelector(s)!, '::after').content,
          selector
        );
      // The first row of each list: the last child never draws one, so the first is the probe.
      expect(await after('#canvas-list .biorouter-settings-row')).toBe('none');
      expect(await after('#muted-list .biorouter-settings-row')).toBe('none');
      // The instrument: the same row outside the catalog draws its rule.
      expect(await after('#canvas-plain .biorouter-settings-row')).not.toBe('none');
    } finally {
      await tab.close();
    }
  });

  it('fills a hovered row and steps the button inside it back one ground, on both grounds', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const fill = await paint(tab, 'var(--background-fill)');
        const strong = await paint(tab, 'var(--background-fill-strong)');
        const page = await paint(tab, 'var(--background-default)');
        for (const [id, rowFill] of [
          ['canvas', fill],
          ['muted', strong],
        ] as const) {
          const row = `#${id}-list [data-testid="provider-card-openai"]`;
          const button = `${row} .br-row-actions button`;
          await tab.mouse.move(690, 1490);
          await tab.waitForTimeout(200);
          // The button rests on the shared control ground, which is the fill: the same value the
          // row would wash to, which is what made it vanish.
          await tab.hover(row);
          await tab.waitForTimeout(250);
          expect(await ground(tab, row), `${mode} ${id} hovered row`).toBe(rowFill);
          expect(await ground(tab, button), `${mode} ${id} the button in a hovered row`).toBe(page);
          expect(rowFill, `${mode} ${id} row differs from the button`).not.toBe(page);
          // And the button still answers the pointer on its own.
          await tab.hover(button);
          await tab.waitForTimeout(250);
          expect(await ground(tab, button), `${mode} ${id} the hovered button`).toBe(strong);
        }
        await tab.mouse.move(690, 1490);
        await shoot(tab, `first-run-row-${mode}`);
      } finally {
        await tab.close();
      }
    }
  });

  it('proves the instrument: without the ground rule the muted page loses its fields', async (ctx) => {
    if (!browser) return ctx.skip();
    const stripped = ONBOARDING_CSS.replace(
      /\.br-first-run-ground\s*\{[^}]*\}/,
      '.br-first-run-ground {}'
    );
    expect(stripped).not.toBe(ONBOARDING_CSS);
    const tab = await open('light', { sheet: css + PROVIDERS_CSS + stripped });
    try {
      const muted = await paint(tab, 'var(--background-muted)');
      expect(await ground(tab, '#muted-field')).toBe(muted);
    } finally {
      await tab.close();
    }
  });

  it('rests a progress track one step deeper than the box it sits in', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      const strong = await paint(tab, 'var(--background-fill-strong)');
      const deeper = await paint(tab, 'var(--background-strong)');
      expect(await ground(tab, '#canvas-progress')).toBe(strong);
      expect(await ground(tab, '#muted-progress')).toBe(deeper);
    } finally {
      await tab.close();
    }
  });
});
