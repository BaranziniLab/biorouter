// @vitest-environment node
/**
 * An accordion provider row's keyboard focus, in a real layout engine (vision FR-R2-3).
 *
 * Llama Server, Ollama, Claude Code, Codex and Versa API Bedrock open in place: the row's identity
 * is one `<button>` (`.br-provider-row-toggle`). By keyboard that button took the base layer's
 * control fallback, `--background-focus` and an inset 2px `--border-focus` edge at radius 0 with no
 * room, so the title and the second line touched a square box drawn inside the row's rounded wash.
 * The row now draws D-15's row focus at its own radius and the toggle draws nothing. jsdom has no
 * `:focus-visible` and no cascade, so the real `ProviderCard` is bundled, the real `main.css` is
 * compiled by Tailwind, `providers.css` is appended as authored, and Chromium is driven with the
 * Tab key.
 *
 * Set `WS_SCREENSHOT_DIR` to also save a PNG in light and dark.
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
import { ProviderCard } from './subcomponents/ProviderCard';

const provider = (name: string, display: string, description: string) => ({
  name,
  is_configured: true,
  provider_type: 'Builtin',
  metadata: {
    name,
    display_name: display,
    description,
    default_model: 'm',
    known_models: [],
    model_doc_link: '',
    config_keys: [],
    tier: 'private',
    runs_locally: true,
    institutions: [],
    allows_unlisted_models: true,
  },
});

function Catalog() {
  const [open, setOpen] = React.useState(false);
  return (
    <div style={{ background: 'var(--background-canvas)', padding: 24, width: 700 }}>
      <button id="before" type="button">before</button>
      <div className="br-provider-catalog br-rows-flat">
        <div className="biorouter-settings-list">
          <ProviderCard
            provider={provider('llamacpp', 'Llama Server', 'Built-in local models (llama.cpp): private, free, no setup required') as never}
            onConfigure={() => {}}
            onLaunch={() => {}}
            isOnboarding
            expandable
            expanded={open}
            onToggle={() => setOpen((v) => !v)}
          >
            <p>Panel</p>
          </ProviderCard>
          <ProviderCard
            provider={provider('ollama', 'Ollama', 'Local open source models') as never}
            onConfigure={() => {}}
            onLaunch={() => {}}
            isOnboarding
            expandable
            expanded={false}
            onToggle={() => {}}
          />
        </div>
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<Catalog />);
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

const ROW = '[data-testid="provider-card-llamacpp"]';
const TOGGLE = '[data-testid="provider-row-toggle-llamacpp"]';

describe('an accordion provider row draws its keyboard focus on the row (FR-R2-3)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the provider row focus tests.');
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

  async function open(mode: 'light' | 'dark', providersCss = PROVIDERS_CSS): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 760, height: 400 } });
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}${providersCss}</style></head>` +
        `<body style="margin:0"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector(ROW);
    // Park the pointer away from the rows, then Tab from the control before them onto the toggle.
    await tab.mouse.move(750, 390);
    await tab.focus('#before');
    await tab.keyboard.press('Tab');
    await tab.waitForTimeout(250);
    return tab;
  }

  /** The computed colour a probe paints for `value` (a token, in this document). */
  const paint = (tab: Page, value: string) =>
    tab.evaluate((v) => {
      const probe = document.createElement('div');
      probe.style.backgroundColor = v;
      document.body.append(probe);
      const out = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return out;
    }, value);
  const style = (tab: Page, selector: string) =>
    tab.evaluate((s) => {
      const cs = getComputedStyle(document.querySelector(s)!);
      return {
        background: cs.backgroundColor,
        shadow: cs.boxShadow,
        radius: parseFloat(cs.borderTopLeftRadius),
      };
    }, selector);
  const edgeColor = (tab: Page) =>
    tab.evaluate(() => {
      const probe = document.createElement('div');
      probe.style.color = 'var(--border-focus)';
      document.body.append(probe);
      const out = getComputedStyle(probe).color;
      probe.remove();
      return out;
    });

  const shoot = async (tab: Page, name: string) => {
    const dir = process.env.WS_SCREENSHOT_DIR;
    if (!dir) return;
    mkdirSync(dir, { recursive: true });
    await tab.screenshot({ path: resolve(dir, `${name}.png`) });
  };

  it('paints the focus on the rounded row and nothing on the toggle', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        expect(
          await tab.evaluate((s) => document.activeElement === document.querySelector(s), TOGGLE),
          `${mode}: Tab lands on the toggle`
        ).toBe(true);
        expect(
          await tab.evaluate((s) => document.querySelector(s)!.matches(':focus-visible'), TOGGLE)
        ).toBe(true);
        const focusFill = await paint(tab, 'var(--background-focus)');
        const edge = await edgeColor(tab);
        const row = await style(tab, ROW);
        expect(row.background, `${mode} row fill`).toBe(focusFill);
        expect(row.shadow, `${mode} row edge`).toContain('inset');
        expect(row.shadow, `${mode} row edge colour`).toContain(edge);
        expect(row.shadow, `${mode} row edge width`).toContain('2px');
        expect(row.radius, `${mode} the row keeps its radius`).toBeGreaterThan(0);

        const toggle = await style(tab, TOGGLE);
        expect(toggle.background, `${mode} toggle fill`).toBe('rgba(0, 0, 0, 0)');
        expect(toggle.shadow, `${mode} toggle edge`).toBe('none');
        await shoot(tab, `provider-row-focus-${mode}`);

        // Under the pointer it still reads as focused (the catalog's hover paint is out-ranked).
        await tab.hover(ROW);
        await tab.waitForTimeout(250);
        expect((await style(tab, ROW)).background, `${mode} focused row under the pointer`).toBe(
          focusFill
        );
      } finally {
        await tab.close();
      }
    }
  });

  it('leaves a row the keyboard is not on without the focus treatment', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      const focusFill = await paint(tab, 'var(--background-focus)');
      const other = await style(tab, '[data-testid="provider-card-ollama"]');
      expect(other.background).not.toBe(focusFill);
      expect(other.shadow).toBe('none');
      // A click (no :focus-visible on a clicked button) leaves no focus treatment either.
      await tab.click('[data-testid="provider-row-toggle-ollama"]');
      await tab.mouse.move(750, 390);
      await tab.waitForTimeout(250);
      const clicked = await style(tab, '[data-testid="provider-card-ollama"]');
      expect(clicked.shadow).toBe('none');
      expect(clicked.background).not.toBe(focusFill);
    } finally {
      await tab.close();
    }
  });

  it('is the providers.css rule that does it: without it the toggle draws a square box', async (ctx) => {
    if (!browser) return ctx.skip();
    const withoutRule = PROVIDERS_CSS.replace(
      /\.biorouter-settings-row\.br-setting-row:has\(> \.br-provider-row-toggle:focus-visible\),[\s\S]*?\.br-provider-row-toggle:focus-visible \{[\s\S]*?\}\n/,
      ''
    );
    expect(withoutRule).not.toBe(PROVIDERS_CSS);
    const tab = await open('light', withoutRule);
    try {
      const focusFill = await paint(tab, 'var(--background-focus)');
      const toggle = await style(tab, TOGGLE);
      expect(toggle.background).toBe(focusFill);
      expect(toggle.radius).toBe(0);
    } finally {
      await tab.close();
    }
  });
});
