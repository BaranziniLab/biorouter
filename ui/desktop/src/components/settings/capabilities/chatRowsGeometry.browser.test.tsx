// @vitest-environment node
/**
 * The Chat tab's capability and memory rows, measured in a real layout engine (spec 3.14, 5.7).
 *
 * jsdom lays nothing out and runs no CSS, so a component test can only say "this class is on this
 * node". The acceptance numbers for this workstream are geometry: every capability row is one
 * 40px line, Biorouter Copilot's check result is a block AFTER its row and never swells it, and a
 * Memory category is one 40px line (a bare 24px chevron once made it 46) whose count sits on the
 * same right edge as its store's, whose entries line up under its name, and whose delete glyph is
 * hidden until the row is pointed at.
 *
 * The real `CapabilitiesSection` and `MemorySection` are bundled with esbuild (their daemon,
 * config and toast dependencies stubbed), rendered by React in Chromium, and styled by the
 * authored `@ws WS-PRIMITIVES` blocks of `styles/main.css` plus `memory.css`, with the token
 * values read from that file. Tailwind is not run, so only the utilities the measured boxes
 * depend on are written out below.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../../styles/main.css'), 'utf8');
const MEMORY_CSS = readFileSync(resolve(here, '../memory/memory.css'), 'utf8');

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
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
.text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
.text-secondary { font-size: 13px; line-height: 18px; }
.text-supporting, .text-caps { font-size: 12px; line-height: 16px; }
.flex { display: flex; } .items-center { align-items: center; } .items-start { align-items: flex-start; }
.flex-1 { flex: 1 1 0%; } .min-w-0 { min-width: 0; } .gap-1 { gap: 4px; } .gap-2 { gap: 8px; }
.gap-3 { gap: 12px; } .flex-wrap { flex-wrap: wrap; } .px-3 { padding-inline: 12px; }
.py-2\\.5 { padding-block: 10px; } .mx-3 { margin-inline: 12px; } .mt-1 { margin-top: 4px; }
.mb-1 { margin-bottom: 4px; } .mb-2 { margin-bottom: 8px; } .rounded-element { border-radius: 8px; }
.biorouter-settings-list { display: flex; flex-direction: column; }
.biorouter-settings-section-header { margin-bottom: 8px; }
.br-switch { flex: none; }
:root { --control-track-off: #ebebe8; --background-accent: #b85a32; --border-control: #86847f;
  --background-default: #fff; --text-on-accent: #fff; --text-muted: #666; --text-default: #111;
  --text-subtle: #777; --text-danger: #b00020; --background-muted: #f2f2f0;
  --border-focus: #555; --background-focus: #ddd;
  --shadow-raised: 0 1px 2px rgb(0 0 0 / 0.1); --inset-hairline: inset 0 0 0 1px #ddd;
  --overlay-hover: rgb(0 0 0 / 0.05); --border-subtle: #ccc; }
`;

/** What `import` specifiers the real components reach for that a page cannot serve. */
const STUBS: Array<{ filter: RegExp; contents: string }> = [
  {
    filter: /\/ConfigContext$/,
    contents: `
      const names = ['developer','computercontroller','webdocuments','autovisualiser','code_execution',
        'extensionmanager','skills','todo','memory','knowledge','agent_drafter','chatrecall','workspace'];
      const extensionsList = names.map((name) => ({ type: 'platform', name, description: name, enabled: true }));
      export const useConfig = () => ({ extensionsList, addExtension: async () => {}, getExtensions: async () => [], config: {}, upsert: async () => {} });
    `,
  },
  {
    filter: /^\.\.\/extensions$/,
    contents: `export const toggleExtensionDefault = async () => {};`,
  },
  {
    filter: /\/copilotApi$/,
    contents: `
      export const copilotSetup = async () => new Promise((resolve) => setTimeout(() => resolve({
        status: 'os_permission_required', runtime_version: '0.3.5', target: 'darwin-arm64',
        permissions: { accessibility: true, screen_recording: false },
        executable: '/Applications/Biorouter.app/Contents/Resources/computer-use/ocu',
        message: 'Screen Recording permission is required.',
      }), 120));
    `,
  },
  {
    filter: /\/api$/,
    contents: `
      const entry = (index, content, tags = []) => ({ index, tags, content, digest: 'd' + index });
      const store = (scope, path, categories) => ({ scope, path, exists: true, categories });
      export const memoryInventory = async () => ({ data: {
        global: store('global', '/home/me/.config/biorouter/memory', [
          { name: 'clinical', entries: [entry(0, 'The cohort has 812 patients.', ['phi']), entry(1, 'Report effect sizes with intervals.')], revision: 'r1', size_bytes: 412, modified: 1780000000 },
          { name: 'tools', entries: [entry(0, 'Use ggplot2.')], revision: 'r2', size_bytes: 24, modified: 1780000000 },
        ]),
        local: store('local', '/proj/.biorouter/memory', [
          { name: 'development', entries: [entry(0, 'We format with black.')], revision: 'r3', size_bytes: 24, modified: 1780000000 },
        ]),
      } });
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
        // Only the specifiers the components under test import; the stub files do not exist.
        args.importer.includes('/settings/') || args.importer.includes('/copilot/')
          ? { path: args.path, namespace: 'stub', pluginData: contents }
          : undefined
      );
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      loader: 'js',
    }));
    // The stylesheet is added to the page by hand.
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { SettingSection } from '../../ui/setting-row';
import { CapabilitiesSection } from './CapabilitiesSection';
import MemorySection from '../memory/MemorySection';

createRoot(document.getElementById('root')!).render(
  <div style={{ width: 712, padding: 16 }}>
    <div id="caps">
      <SettingSection title="Capabilities" help="Choose what new chats start with.">
        <CapabilitiesSection />
      </SettingSection>
    </div>
    <div id="memory">
      <MemorySection />
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

const EPSILON = 0.5;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

describe('the Chat tab rows in a real layout engine (spec 3.14, 5.7)', () => {
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
    css = PREFLIGHT + readTokens() + primitiveBlocks() + MEMORY_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Let every entrance finish: `.br-enter` slides 4px, and a rect read mid-slide is 4px off. */
  const settle = (tab: Page) =>
    tab.evaluate(() =>
      Promise.all(document.getAnimations().map((animation) => animation.finished))
    );

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 1400 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    // Memory loads asynchronously: wait for a category row.
    await tab.waitForSelector('#memory .br-memory-toggle');
    await tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );
    return tab;
  }

  it('draws every capability as one 40px line', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const heights = await tab.evaluate(() =>
        [...document.querySelectorAll('#caps .biorouter-settings-row')].map(
          (row) => row.getBoundingClientRect().height
        )
      );
      expect(heights).toHaveLength(14);
      for (const height of heights) near(height, 40, 'capability row');
    } finally {
      await tab.close();
    }
  });

  it('puts the Copilot check result after its row without growing the row', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const rowOf = () =>
        tab.evaluate(() => {
          const rows = [...document.querySelectorAll('#caps .biorouter-settings-row')];
          const row = rows.find(
            (r) => r.querySelector('label')?.textContent === 'Biorouter Copilot'
          )!;
          const panel = document.getElementById('copilot-setup-details');
          const next =
            row.nextElementSibling === panel ? panel!.nextElementSibling : row.nextElementSibling;
          const box = (el: Element | null) => (el ? el.getBoundingClientRect().toJSON() : null);
          return { row: box(row), panel: box(panel), next: box(next) };
        });
      const before = await rowOf();
      near(before.row.height, 40, 'Copilot row before the check');
      expect(before.panel).toBeNull();

      await tab.getByRole('button', { name: 'Check setup' }).click();
      await tab.waitForSelector('#copilot-setup-details');
      await settle(tab);
      const after = await rowOf();

      near(after.row.height, 40, 'Copilot row with the result open');
      expect(after.panel!.height).toBeGreaterThan(80);
      expect(after.panel!.top).toBeGreaterThanOrEqual(after.row.bottom - EPSILON);
      expect(after.next!.top).toBeGreaterThanOrEqual(after.panel!.bottom - EPSILON);
      // Inset like the rows' own text, not the full width of the list.
      near(after.panel!.left - after.row.left, 12, 'result block inset');
    } finally {
      await tab.close();
    }
  });

  // A `disabled` button that held keyboard focus hands it to <body>, so a person who pressed Enter
  // on "Check setup" was thrown back to the top of the page. jsdom does not run that focus fixup.
  it('keeps keyboard focus on the Copilot check button while the check runs and after it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const focused = () =>
        tab.evaluate(() => {
          const el = document.activeElement;
          return el && el !== document.body
            ? { text: el.textContent?.trim(), busy: el.getAttribute('aria-busy') }
            : null;
        });
      await tab.getByRole('button', { name: 'Check setup' }).focus();
      await tab.keyboard.press('Enter');
      // Mid-check: the button says it is working and still holds focus.
      await tab.waitForSelector('#caps button[aria-busy="true"]');
      expect(await focused()).toEqual({ text: 'Checking…', busy: 'true' });
      await tab.waitForSelector('#copilot-setup-details');
      await settle(tab);
      expect(await focused()).toEqual({ text: 'Hide setup', busy: 'false' });

      // And "Check again" inside the result keeps it too.
      await tab.getByRole('button', { name: 'Check again' }).focus();
      await tab.keyboard.press('Enter');
      await tab.waitForSelector('#copilot-setup-details button[aria-busy="true"]');
      expect((await focused())?.busy).toBe('true');
      await tab.waitForSelector('#copilot-setup-details button[aria-busy="false"]');
      expect((await focused())?.text).toBe('Check again');
    } finally {
      await tab.close();
    }
  });

  it('draws a Memory category as one 40px line with a 16px chevron', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const rows = [...document.querySelectorAll('#memory .biorouter-settings-row')];
        return {
          heights: rows.map((r) => r.getBoundingClientRect().height),
          chevron: document
            .querySelector('#memory .br-memory-toggle > svg')!
            .getBoundingClientRect()
            .toJSON(),
        };
      });
      expect(m.heights).toHaveLength(3);
      for (const height of m.heights) near(height, 40, 'category row');
      near(m.chevron.width, 16, 'chevron width');
      near(m.chevron.height, 16, 'chevron height');
    } finally {
      await tab.close();
    }
  });

  // The disclosure button owns its focus box: a 28px rounded control, not a 20px square hug of the
  // text with the chevron touching its edge. Its padding is cancelled by a negative margin, so the
  // chevron and the name stay where the row's 12px inset puts them and the row stays 40px.
  it('gives the category button a 28px rounded focus box without moving the chevron or the name', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const rect = (el: Element | null) => el!.getBoundingClientRect().toJSON();
        const toggle = document.querySelector('#memory .br-memory-toggle')!;
        const row = toggle.closest('.biorouter-settings-row')!;
        return {
          row: rect(row),
          toggle: rect(toggle),
          chevron: rect(toggle.querySelector('svg')),
          name: rect(toggle.querySelector('span')),
          radius: getComputedStyle(toggle).borderTopLeftRadius,
        };
      });
      near(m.toggle.height, 28, 'focus box height');
      expect(m.radius, 'focus box radius').toBe('8px');
      near(m.row.height, 40, 'category row');
      // The box reaches 8px past the chevron; the chevron and name keep the old positions.
      near(m.chevron.left - m.toggle.left, 8, 'room left of the chevron');
      near(m.chevron.left - m.row.left, 12, 'chevron inset from the row');
      near(m.name.left - m.chevron.left, 16 + 8, 'name after the chevron and its gap');
      // Centred on the row's line, within the 1px transparent edge the row keeps.
      expect(m.toggle.top - m.row.top).toBeGreaterThanOrEqual(4);
      expect(m.toggle.top - m.row.top).toBeLessThanOrEqual(8);
    } finally {
      await tab.close();
    }
  });

  it('lines the counts up on one right edge and the entries up under the name', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.getByRole('button', { name: /show the 2 memories in clinical/i }).click();
      await tab.waitForSelector('#memory .br-memory-entry');
      await settle(tab);
      const m = await tab.evaluate(() => {
        const rect = (el: Element | null) => el!.getBoundingClientRect().toJSON();
        const category = document.querySelector('#memory .br-memory-toggle')!;
        const name = category.querySelector('span')!;
        const entryText = document.querySelector('#memory .br-memory-entry p')!;
        return {
          name: rect(name),
          entryText: rect(entryText),
          storeCount: rect(document.querySelector('#memory .br-memory-store-count')),
          meta: rect(document.querySelector('#memory .br-row-actions-meta')),
          category: rect(category.closest('.biorouter-settings-row')),
          entry: rect(document.querySelector('#memory .br-memory-entry')),
        };
      });
      // The store's count and the category's count end on the same line (12px in from the list).
      near(m.storeCount.right, m.meta.right, 'store count against category count');
      // An entry's text starts where the category's name does.
      near(m.entryText.left, m.name.left, 'entry text under the category name');
      // An entry is its own row, directly under the category row, not inside it.
      near(m.entry.top, m.category.bottom, 'entry row under the category row');
    } finally {
      await tab.close();
    }
  });

  it('hides the delete glyph until the row is pointed at', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const opacity = () =>
        tab.evaluate(() => {
          const row = document.querySelector('#memory .biorouter-settings-row')!;
          return {
            cluster: Number(getComputedStyle(row.querySelector('.br-row-actions')!).opacity),
            meta: Number(getComputedStyle(row.querySelector('.br-row-actions-meta')!).opacity),
          };
        });
      expect(await opacity()).toEqual({ cluster: 0, meta: 1 });

      await tab.locator('#memory .biorouter-settings-row').first().hover();
      await tab.waitForTimeout(250);
      expect(await opacity()).toEqual({ cluster: 1, meta: 0 });

      // And it is muted until the glyph itself is pointed at, then danger ink.
      const ink = async () =>
        tab.evaluate(() => {
          const button = document.querySelector('#memory .br-memory-delete')!;
          return getComputedStyle(button).color;
        });
      const resting = await ink();
      await tab.locator('#memory .br-memory-delete').first().hover();
      await tab.waitForTimeout(250);
      expect(await ink()).not.toBe(resting);
    } finally {
      await tab.close();
    }
  });
});
