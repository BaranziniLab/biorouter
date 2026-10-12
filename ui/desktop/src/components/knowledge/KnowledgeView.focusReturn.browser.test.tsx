// @vitest-environment node
/**
 * Where keyboard focus lands when the knowledge-base manager is closed, in a real browser
 * (V3-R7-3).
 *
 * `KnowledgeView.focusReturn.test.tsx` pins the semantics in jsdom, but jsdom has no exit
 * animation: the picker's popover unmounts the instant its row acts, so focus is already back on
 * the trigger when the dialog opens. In Chromium the popover is still mounted, playing its close
 * motion, when the manager mounts, so the dialog reads the picker's search field as its opener,
 * which is gone by the time the dialog closes, and Escape left focus on `<body>` (measured in the
 * running app: Enter on the trigger, ArrowDown, Enter on "Manage bases…", Escape, `activeElement`
 * is `BODY` with no dialog open). The same on the "Create knowledge base…" row, where Escape from
 * the chooser lands in the manager and the next Escape landed on `<body>`.
 *
 * This lays the real view out in Chromium, with the real selector, picker, manager and chooser,
 * `styles/main.css` compiled by Tailwind over the classes the bundle uses, and the real exit
 * motion (no `reducedMotion`), and presses keys with the page's own keyboard.
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

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import KnowledgeView from './KnowledgeView';

createRoot(document.getElementById('root')!).render(<KnowledgeView />);
`;

/** What the view reads from the app: two bases, and clients that are never reached. */
const BASES = `[
  { id: 'omop', name: 'OMOP', color: '#cf6d47', tier: 'public' },
  { id: 'soul', name: 'Soul', color: '#b85a32', tier: 'public' },
]`;

const STUBS: Record<string, string> = {
  KnowledgeContext: `
    const bases = ${BASES};
    const value = {
      bases,
      visibleBases: bases,
      primaryKb: bases[0],
      primaryKbId: 'omop',
      defaultPrimaryKb: null,
      canFollowDefaultPrimary: false,
      followDefaultPrimary: () => {},
      hiddenKbIds: [],
      loading: false,
      refresh: async () => {},
      refreshDefaultPrimary: async () => {},
      registerGraphRefresh: () => {},
      setPrimaryKbId: () => {},
      toggleKbHidden: () => {},
    };
    export const useKnowledge = () => value;
  `,
  useKnowledgeBases: `
    const value = {
      create: async () => ({ id: 'x' }),
      exportArchive: async () => {},
      importArchive: async () => {},
      remove: async () => {},
      rename: async () => {},
    };
    export const useKnowledgeBases = () => value;
  `,
  useKnowledgeGraph: `
    const value = { graph: { nodes: [], edges: [] }, loading: false, error: null, refresh: async () => {} };
    export const useKnowledgeGraph = () => value;
  `,
  KbTierControl: `export const KbTierPanel = () => null;`,
  SourcesRail: `export const SourcesRail = () => null;`,
  KnowledgeGraphPanel: `export const KnowledgeGraphPanel = () => null;`,
  ChangeLogDrawer: `export const ChangeLogDrawer = () => null;`,
  LintDrawer: `export const LintDrawer = () => null;`,
  MainPanelLayout: `export const MainPanelLayout = ({ children }) => <div style={{ display: 'flex', height: '100vh' }}>{children}</div>;`,
};

const stubPlugin: Plugin = {
  name: 'knowledge-view-stubs',
  setup(b) {
    const names = Object.keys(STUBS).join('|');
    b.onResolve({ filter: new RegExp(`/(${names})$`) }, (args) => {
      const name = Object.keys(STUBS).find((key) => args.path.endsWith(`/${key}`));
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

const TRIGGER = '[data-testid="knowledge-kb-selector-trigger"]';
const MANAGE = '[data-testid="knowledge-kb-open-manager"]';
const CREATE = '[data-testid="knowledge-kb-open-create"]';

describe('closing the knowledge-base manager (Chromium)', () => {
  let browser: Browser | null = null;
  let bundle = { script: '', css: '' };

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the focus return tests.');
      return;
    }
    const built = await build({
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
    const script = built.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    bundle = {
      script,
      css: compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])),
    };
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width: 1200, height: 800 } });
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.setContent(
      `<!doctype html><html><head><style>${bundle.css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: bundle.script });
    await page.waitForSelector(TRIGGER);
    expect(errors, 'page errors').toEqual([]);
    return page;
  }

  /** What holds focus, named the way a reader of a failure wants it. */
  const focused = (page: Page): Promise<string> =>
    page.evaluate(() => {
      const el = document.activeElement as HTMLElement | null;
      if (!el || el === document.body) return 'BODY';
      return el.getAttribute('data-testid') ?? el.getAttribute('aria-label') ?? el.tagName;
    });

  const dialogCount = (page: Page): Promise<number> =>
    page.evaluate(() => document.querySelectorAll('[role="dialog"]').length);

  /** Enter on the focused trigger, ArrowDown to the row, Enter on it: keys only, no pointer. */
  async function keyboardPick(page: Page, row: string) {
    await page.focus(TRIGGER);
    await page.keyboard.press('Enter');
    await page.waitForSelector(row);
    for (let step = 0; step < 6; step += 1) {
      const onRow = await page.evaluate((selector) => {
        const combobox = document.querySelector('[role="combobox"]');
        const id = combobox?.getAttribute('aria-activedescendant');
        return Boolean(id && document.getElementById(id) === document.querySelector(selector));
      }, row);
      if (onRow) break;
      await page.keyboard.press('ArrowDown');
    }
    await page.keyboard.press('Enter');
  }

  const settled = async (page: Page, count: number) => {
    await page.waitForFunction(
      (expected) => document.querySelectorAll('[role="dialog"]').length === expected,
      count
    );
    // The close motion and the focus scope's own deferred return have both played.
    await page.waitForTimeout(400);
  };

  it('gives focus back to the selector after a keyboard pick on Manage bases and Escape', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open();
    try {
      await keyboardPick(page, MANAGE);
      await settled(page, 1);
      expect(await focused(page)).not.toBe('BODY');

      await page.keyboard.press('Escape');
      await settled(page, 0);
      expect(await focused(page)).toBe('knowledge-kb-selector-trigger');
    } finally {
      await page.close();
    }
  });

  it('gives it back from Create too: Escape returns to the manager, the next Escape to the selector', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open();
    try {
      await keyboardPick(page, CREATE);
      await settled(page, 1);
      expect(
        await page.evaluate(() => document.querySelector('[role="dialog"]')?.textContent)
      ).toContain('Create knowledge base');

      await page.keyboard.press('Escape');
      await settled(page, 1);
      expect(await focused(page)).toBe('knowledge-kb-create');

      await page.keyboard.press('Escape');
      await settled(page, 0);
      expect(await focused(page)).toBe('knowledge-kb-selector-trigger');
    } finally {
      await page.close();
    }
  });

  it('does not move focus to the selector after a pointer pick', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open();
    try {
      await page.click(TRIGGER);
      await page.click(MANAGE);
      await settled(page, 1);

      await page.keyboard.press('Escape');
      await settled(page, 0);
      expect(await focused(page)).not.toBe('knowledge-kb-selector-trigger');
      expect(await dialogCount(page)).toBe(0);
    } finally {
      await page.close();
    }
  });
});
