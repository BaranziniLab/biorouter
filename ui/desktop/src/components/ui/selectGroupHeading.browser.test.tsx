// @vitest-environment node
/**
 * A Select menu of one unnamed group starts where a flat list does (V4-R8-1).
 *
 * Switch models stacks two Selects: Provider passes a flat list and Model passes its options as one
 * unlabeled group (`[{ options }]`). react-select draws a `GroupHeading` for every group, label or
 * not, and the shared heading class (`MENU_GROUP_LABEL_CLASS_NAME`: 6px above, 2px below, 16px
 * line) made an empty 8px row above the Model menu's first option: 13px below the surface's top
 * against the Provider menu's 5px (the 1px border and the 4px list padding), in the same dialog.
 * Spec 2.6 puts a group label only where a group is named.
 *
 * Layout is measured in Chromium with `styles/main.css` compiled by Tailwind (jsdom has no layout
 * engine). The fixtures use the real shapes: a flat list, one unlabeled group with the model
 * picker's two-line option label and a portalled menu (as the modal's Model field is), and named
 * groups. A fourth Select that puts the library's own `GroupHeading` back is the instrument: it
 * must read the unfixed 13px, so the equality below cannot pass because nothing was measured.
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
import { components as selectComponents } from 'react-select';
import { Select } from './Select';

const providers = [
  { value: 'ollama', label: 'Ollama' },
  { value: 'openai', label: 'OpenAI' },
  { value: 'anthropic', label: 'Anthropic' },
];

const models = [
  { value: 'gpt-4o', label: 'gpt-4o', detail: 'Fast, multimodal' },
  { value: 'gpt-4o-mini', label: 'gpt-4o-mini', detail: 'Small and cheap' },
  { value: 'o3', label: 'o3', detail: 'Reasoning' },
];

const named = [
  { label: 'Local', options: [{ value: 'a', label: 'Llama Server' }, { value: 'b', label: 'Ollama' }] },
  { label: 'Hosted', options: [{ value: 'c', label: 'OpenAI' }, { value: 'd', label: 'Anthropic' }] },
];

const twoLine = (option) => (
  <div>
    <div>{option.label}</div>
    <div className="text-supporting text-text-muted">{option.detail}</div>
  </div>
);

function App() {
  return (
    <div style={{ padding: 24, display: 'grid', gap: 24, width: 360 }}>
      <Select classNamePrefix="provider" aria-label="Provider" options={providers} menuPlacement="bottom" />
      <Select
        classNamePrefix="model"
        aria-label="Model"
        options={[{ options: models }]}
        formatOptionLabel={twoLine}
        menuPlacement="bottom"
        menuPortalTarget={document.body}
      />
      <Select classNamePrefix="named" aria-label="Named" options={named} menuPlacement="bottom" />
      <Select
        classNamePrefix="library"
        aria-label="Library heading"
        options={[{ options: models }]}
        formatOptionLabel={twoLine}
        menuPlacement="bottom"
        components={{ GroupHeading: selectComponents.GroupHeading }}
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

interface MenuShape {
  /** First option's top below the surface's outer top edge. */
  firstOptionOffset: number;
  borderTop: number;
  listPaddingTop: number;
  headings: string[];
  headingCount: number;
  optionCount: number;
}

describe('a Select menu of one unnamed group starts like a flat list (V4-R8-1)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Select group heading tests.');
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
    const tab = await browser!.newPage({ viewport: { width: 600, height: 900 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('.provider__control');
    return tab;
  }

  /** Open one Select by its class prefix and measure its menu, then close it. */
  async function measure(tab: Page, prefix: string): Promise<MenuShape> {
    await tab.click(`.${prefix}__control`);
    await tab.waitForSelector(`.${prefix}__menu`);
    // The menu plays a short entrance; read the layout once it has finished moving.
    await tab.waitForTimeout(400);
    const shape = await tab.evaluate((p): MenuShape => {
      const menu = document.querySelector<HTMLElement>(`.${p}__menu`)!;
      const list = menu.querySelector<HTMLElement>(`.${p}__menu-list`)!;
      const first = menu.querySelector<HTMLElement>(`.${p}__option`)!;
      const headings = Array.from(menu.querySelectorAll<HTMLElement>(`.${p}__group-heading`));
      const listStyle = getComputedStyle(list);
      return {
        firstOptionOffset: first.getBoundingClientRect().top - menu.getBoundingClientRect().top,
        borderTop: parseFloat(getComputedStyle(menu).borderTopWidth),
        listPaddingTop: parseFloat(listStyle.paddingTop),
        headings: headings.map((h) => h.textContent ?? ''),
        headingCount: headings.length,
        optionCount: menu.querySelectorAll(`.${p}__option`).length,
      };
    }, prefix);
    await tab.keyboard.press('Escape');
    await tab.waitForSelector(`.${prefix}__menu`, { state: 'detached' });
    return shape;
  }

  it('the Provider menu (a flat list) starts 5px below its surface: 1px border, 4px padding', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const provider = await measure(tab, 'provider');
      expect(provider.optionCount).toBe(3);
      expect(provider.headingCount).toBe(0);
      expect(provider.borderTop + provider.listPaddingTop).toBe(5);
      expect(provider.firstOptionOffset).toBe(5);
    } finally {
      await tab.close();
    }
  });

  it('the Model menu (one unlabeled group, portalled) draws no heading and starts at the same 5px', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const provider = await measure(tab, 'provider');
      const model = await measure(tab, 'model');
      expect(model.optionCount).toBe(3);
      expect(model.headingCount).toBe(0);
      expect(model.firstOptionOffset).toBe(provider.firstOptionOffset);
      expect(model.firstOptionOffset).toBe(5);
    } finally {
      await tab.close();
    }
  });

  it('a group that has a name still shows it, and its first option sits below it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const provider = await measure(tab, 'provider');
      const shape = await measure(tab, 'named');
      expect(shape.headings).toEqual(['Local', 'Hosted']);
      expect(shape.optionCount).toBe(4);
      // The heading is the 12/16 label plus its 6px above and 2px below: 24px under the 5px edge.
      expect(shape.firstOptionOffset).toBe(provider.firstOptionOffset + 24);
    } finally {
      await tab.close();
    }
  });

  it('the instrument: the library’s own heading in the same unnamed group reads the unfixed 13px', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const library = await measure(tab, 'library');
      expect(library.headingCount).toBe(1);
      expect(library.headings).toEqual(['']);
      expect(library.firstOptionOffset).toBe(13);
    } finally {
      await tab.close();
    }
  });
});
