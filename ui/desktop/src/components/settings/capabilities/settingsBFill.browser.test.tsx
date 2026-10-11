// @vitest-environment node
/**
 * The Chat tab's Copilot result and the Configuration editor under the fill rule, in a real layout
 * engine (owner message 13, R-27, design.md 9.11; spec 3.14).
 *
 * jsdom runs no CSS and no Tailwind, so a component test can only read class names. Every claim
 * here is about what the cascade paints: a block that is a fill with a transparent edge, a control
 * that steps back one ground inside it, a field whose unsaved edit is a wash and not an outline,
 * and a focused field that still shows its focus edge. So the real `CapabilitiesSection` and
 * `ConfigSettings` are bundled with esbuild (their daemon, config and toast dependencies stubbed),
 * the app's own `styles/main.css` is compiled by Tailwind with the classes the bundle uses (plus
 * the two sheets a component import would have carried), and Chromium paints them. Colours are
 * compared with probes that paint the token itself, so a token value can change without this file
 * changing.
 *
 * Set `SETTINGS_B_SHOTS` to a directory to also write a screenshot of each surface, light and dark.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../../styles');
const MEMORY_CSS = readFileSync(resolve(here, '../memory/memory.css'), 'utf8');
const CONFIG_CSS = readFileSync(resolve(here, '../config/config.css'), 'utf8');
const requireFromDesktop = createRequire(resolve(here, '../../../../package.json'));
const SHOTS = process.env.SETTINGS_B_SHOTS;

/**
 * What the components import that a bare page cannot serve. Every function is created once, at
 * module level: the real provider's callbacks keep their identity, and a stub that handed back a
 * fresh closure per render re-fired `useEffect([refreshConfig])` on every render and looped.
 */
const STUBS: Array<{ filter: RegExp; contents: string }> = [
  {
    filter: /\/ConfigContext$/,
    contents: `
      const names = ['developer','computercontroller','webdocuments','autovisualiser','code_execution',
        'extensionmanager','skills','todo','memory','knowledge','agent_drafter','chatrecall','workspace'];
      const extensionsList = names.map((name) => ({ type: 'platform', name, description: name, enabled: true }));
      const config = {
        BIOROUTER_MODE: 'smart_approve',
        BIOROUTER_MAX_TURNS: 100,
        BIOROUTER_PROVIDER: 'versa_azure',
        VERSA_AZURE_HOST: 'https://versa.example.edu',
        SOME_REPORT: { checked: 3, failed: 0 },
      };
      const api = {
        extensionsList, config,
        addExtension: async () => {}, getExtensions: async () => [], upsert: async () => {},
        refreshConfig: async () => {}, getProviders: async () => [],
      };
      export const useConfig = () => api;
    `,
  },
  {
    filter: /\/ModelAndProviderContext$/,
    contents: `
      const value = { currentProvider: 'versa_azure', refreshCurrentModelAndProvider: async () => {} };
      export const useModelAndProvider = () => value;
    `,
  },
  {
    filter: /^\.\.\/extensions$/,
    contents: `export const toggleExtensionDefault = async () => {};`,
  },
  {
    filter: /\/copilotApi$/,
    contents: `
      export const copilotSetup = async () => ({
        status: 'os_permission_required', runtime_version: '0.3.5', target: 'darwin-arm64',
        permissions: { accessibility: true, screen_recording: false },
        executable: '/Applications/Biorouter.app/Contents/Resources/computer-use/ocu',
        message: 'Screen Recording permission is required.',
      });
    `,
  },
  {
    filter: /\/api$/,
    contents: `
      export const memoryInventory = async () => ({ data: { global: null, local: null } });
      export const memoryDeleteCategory = async () => ({ data: {} });
      export const memoryDeleteEntry = async () => ({ data: {} });
    `,
  },
  {
    filter: /\/toasts$/,
    contents: `export const toastError = () => {}; export const toastSuccess = () => {};`,
  },
  { filter: /\/workingDir$/, contents: `export const getInitialWorkingDir = () => '/proj';` },
];

const stubPlugin: Plugin = {
  name: 'stubs',
  setup(b) {
    for (const { filter, contents } of STUBS) {
      b.onResolve({ filter }, (args) =>
        args.importer.includes('/settings/') || args.importer.includes('/copilot/')
          ? { path: args.path, namespace: 'stub', pluginData: contents }
          : undefined
      );
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      loader: 'js',
    }));
    // The stylesheets are compiled and added to the page by hand.
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { SettingSection } from '../../ui/setting-row';
import { CapabilitiesSection } from './CapabilitiesSection';
import ConfigSettings from '../config/ConfigSettings';

createRoot(document.getElementById('root')!).render(
  <div style={{ width: 712, padding: 16, background: 'var(--background-default)' }}>
    <div id="caps">
      <SettingSection title="Capabilities">
        <CapabilitiesSection />
      </SettingSection>
    </div>
    <div id="config">
      <ConfigSettings />
    </div>
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

describe('Copilot result and the Configuration editor under the fill rule (R-27)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Settings B fill tests.');
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
      loader: { '.svg': 'dataurl', '.png': 'dataurl' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(
      `${readFileSync(resolve(STYLES, 'main.css'), 'utf8')}\n${MEMORY_CSS}\n${CONFIG_CSS}`,
      { base: STYLES, onDependency() {} }
    );
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(mode: 'light' | 'dark'): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 1000 } });
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-canvas)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#configuration-edit');
    await tab.waitForTimeout(200);
    return tab;
  }

  /** The colour a probe paints for a token. `property` picks the background or an edge. */
  const paint = (tab: Page, value: string, property: 'background' | 'border' = 'background') =>
    tab.evaluate(
      ([v, p]) => {
        const probe = document.createElement('div');
        if (p === 'border') probe.style.border = `1px solid ${v}`;
        else probe.style.backgroundColor = v;
        document.body.append(probe);
        const cs = getComputedStyle(probe);
        const out = p === 'border' ? cs.borderTopColor : cs.backgroundColor;
        probe.remove();
        return out;
      },
      [value, property] as const
    );
  /** The computed ink a probe paints for a text token. */
  const paintColor = (tab: Page, token: string) =>
    tab.evaluate((t) => {
      const probe = document.createElement('span');
      probe.style.color = `var(${t})`;
      document.body.append(probe);
      const out = getComputedStyle(probe).color;
      probe.remove();
      return out;
    }, token);
  const ground = (tab: Page, selector: string) =>
    tab
      .locator(selector)
      .first()
      .evaluate((el) => getComputedStyle(el).backgroundColor);
  /** The top edge a box draws: its width and colour. A transparent edge is a border that draws nothing. */
  const edge = (tab: Page, selector: string) =>
    tab
      .locator(selector)
      .first()
      .evaluate((el) => {
        const cs = getComputedStyle(el);
        return { width: cs.borderTopWidth, style: cs.borderTopStyle, color: cs.borderTopColor };
      });
  const drawsNothing = (e: { width: string; style: string; color: string }) =>
    e.style === 'none' || e.width === '0px' || e.color === 'rgba(0, 0, 0, 0)';

  /** Let every transition finish: a field eases its fill over 95ms, and a read mid-way is off. */
  const settle = (tab: Page) =>
    tab.evaluate(() =>
      Promise.all(document.getAnimations().map((animation) => animation.finished))
    );

  async function openEditor(tab: Page) {
    await tab.click('#configuration-edit');
    await tab.waitForSelector('[role="dialog"] .br-config-row');
    await tab.waitForTimeout(300);
  }

  async function shot(tab: Page, name: string) {
    if (!SHOTS) return;
    mkdirSync(SHOTS, { recursive: true });
    await tab.screenshot({ path: resolve(SHOTS, `${name}.png`) });
  }

  it('shows the Copilot result as a fill with no outline, and the button inside it on the page ground', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const fill = await paint(tab, 'var(--background-fill)');
        const page = await paint(tab, 'var(--background-default)');
        const trigger = '#caps .biorouter-settings-row button:has-text("Check setup")';
        expect(await ground(tab, trigger), `${mode} the row's button rests on the fill`).toBe(fill);
        expect(drawsNothing(await edge(tab, trigger)), `${mode} button edge`).toBe(true);

        await tab.click(trigger);
        await tab.waitForSelector('#copilot-setup-details');
        await tab.waitForTimeout(300);
        expect(await ground(tab, '#copilot-setup-details'), `${mode} result block`).toBe(fill);
        expect(drawsNothing(await edge(tab, '#copilot-setup-details')), `${mode} block edge`).toBe(
          true
        );
        // A fill never sits on a fill: "Check again" steps back to the page.
        expect(
          await ground(tab, '#copilot-setup-details button'),
          `${mode} Check again steps back one ground`
        ).toBe(page);
        expect(page).not.toBe(fill);
        // The state line leads the block in the default ink; the lines under it stay muted.
        const inks = await tab.evaluate(() =>
          [...document.querySelectorAll('#copilot-setup-details p')]
            .slice(0, 2)
            .map((p) => getComputedStyle(p).color)
        );
        expect(inks[0], `${mode} state line ink`).toBe(await paintColor(tab, '--text-default'));
        expect(inks[1], `${mode} detail line ink`).toBe(await paintColor(tab, '--text-muted'));
        await shot(tab, `settings-b-copilot-${mode}`);
      } finally {
        await tab.close();
      }
    }
  }, 120_000);

  it('rests the editor’s fields on the fill with no outline, and marks an unsaved edit with a wash', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        await openEditor(tab);
        const fill = await paint(tab, 'var(--background-fill)');
        const wash = await paint(tab, 'var(--wash-info)');
        const focusEdge = await paint(tab, 'var(--border-focus)', 'border');
        // Not the first field: the dialog focuses its first focusable on open, and a focused field
        // draws its focus edge.
        const plain = '#config-field-VERSA_AZURE_HOST';
        const turns = '#config-field-BIOROUTER_MAX_TURNS';

        expect(await ground(tab, plain), `${mode} resting field`).toBe(fill);
        expect(drawsNothing(await edge(tab, plain)), `${mode} resting field edge`).toBe(true);

        // An edit that is not saved: the info wash, still no outline.
        await tab.click(turns);
        await tab.keyboard.press('End');
        await tab.keyboard.type('5');
        await tab.click(plain); // leave it, so `turns` is modified and unfocused
        await settle(tab);
        expect(await ground(tab, turns), `${mode} modified field`).toBe(wash);
        expect(drawsNothing(await edge(tab, turns)), `${mode} modified field edge`).toBe(true);
        expect(wash).not.toBe(fill);

        // Focused again, the modified field still shows the focus edge (it used to be replaced by
        // the modified marker's own outline, so focus was invisible on an edited field).
        await tab.focus(turns);
        await settle(tab);
        const focused = await edge(tab, turns);
        expect(focused.color, `${mode} a focused modified field`).toBe(focusEdge);
        expect(focused.width).toBe('1px');
        await shot(tab, `settings-b-editor-${mode}`);
      } finally {
        await tab.close();
      }
    }
  }, 120_000);

  it('draws a structured value as a fill well with a transparent edge, not the muted ground', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        await openEditor(tab);
        const well = '[data-testid="config-readonly-SOME_REPORT"]';
        expect(await ground(tab, well), `${mode} read-only value`).toBe(
          await paint(tab, 'var(--background-fill)')
        );
        const e = await edge(tab, well);
        expect(e.width, `${mode} the 1px edge is kept`).toBe('1px');
        expect(drawsNothing(e), `${mode} and draws nothing`).toBe(true);
      } finally {
        await tab.close();
      }
    }
  }, 120_000);

  it('keeps the editor’s edges for forced colours: the fields and the well still have a border box', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      await tab.emulateMedia({ forcedColors: 'active' });
      await openEditor(tab);
      for (const selector of [
        '#config-field-VERSA_AZURE_HOST',
        '[data-testid="config-readonly-SOME_REPORT"]',
      ]) {
        const e = await edge(tab, selector);
        expect(e.width, `${selector} keeps its 1px border`).toBe('1px');
        expect(e.style, `${selector} border style`).toBe('solid');
      }
    } finally {
      await tab.close();
    }
  }, 120_000);
});
