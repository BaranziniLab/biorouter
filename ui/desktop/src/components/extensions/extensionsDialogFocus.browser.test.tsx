// @vitest-environment node
/**
 * Keyboard focus comes back to Add when a dialog opened from its menu closes, in a real browser
 * (vision round 3, V3-R3-2).
 *
 * jsdom cannot see this: Radix decides whether to announce "the content mounted, take focus" by
 * asking the browser whether focus is already inside it, and a field that carries `autoFocus`
 * (the marketplace's search box, before `baam/focusFieldOnOpen`) has taken it by the time Radix
 * asks. The dialog then never records which control opened it (`useReturnFocusToOpener`), and
 * Escape left focus on `<body>`: measured in the running app, twice, for Add > Browse
 * marketplace. "Install from file (.brxt)" has no such field and always returned to Add, which is
 * how the page looked half fixed. The repair is in the shared `MarketplaceDialog` (WS-SKILLS,
 * V3-R3-1: the same defect on Skills' Browse and Add dialogs); this file pins the Extensions
 * page's three mounts so the page cannot lose it without a failing case.
 *
 * The REAL `ExtensionsView` is bundled with esbuild and rendered in Chromium, with the real
 * Browse, Install from file and Custom extension dialogs. Only the data edges are stubbed (the
 * config hook, the extension list and the marketplace feed). Each case drives the keyboard the
 * way the finding does: focus Add, Enter, arrow to the item, Enter, Escape. A pick or a close
 * made with the pointer gives no focus back, which is the primitive's rule (a script-focused Add
 * would wear a keyboard ring after a mouse click), and two cases pin it.
 *
 * "The instrument" proves a case can fail: Browse is also read with the dialog primitive's
 * "return focus to the opener" removed, which must leave focus on `<body>`. A mutation that
 * matches nothing throws, so a refactor cannot turn the case into a no-op.
 */
import { readFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { EXTENSIONS_COPY } from './copy';

const here = dirname(fileURLToPath(import.meta.url));

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import ExtensionsView from './ExtensionsView';

createRoot(document.getElementById('root')!).render(
  <ExtensionsView onClose={() => {}} setView={() => {}} viewOptions={{}} />
);
`;

/** Every module the page reaches for data, replaced by a stub. Order matters: first match wins. */
const STUBS: Array<[RegExp, string]> = [
  [
    /\/ConfigContext$/,
    `
    const value = {
      extensionsList: [],
      addExtension: async () => {},
      removeExtension: async () => {},
      getExtensions: async () => [],
      read: async () => null,
      getProviders: async () => [],
    };
    export const useConfig = () => value;
    export const usePrivacyTiersEnabled = () => true;
  `,
  ],
  [
    /settings\/extensions\/ExtensionsSection$/,
    `
    export default function ExtensionsSection() { return null; }
    export const listedExtensions = () => [];
  `,
  ],
  [
    /(^|\/)settings\/extensions$/,
    `
    export const activateExtensionDefault = async () => {};
    export const toggleExtensionDefault = async () => {};
    export const deleteExtension = async () => {};
  `,
  ],
  [/\/ExtensionLoadFailureNotice$/, `export const ExtensionLoadFailureNotice = () => null;`],
  [/\/sessions$/, `export const startNewSession = async () => {};`],
];

async function bundle(stripFocusReturn: boolean): Promise<string> {
  const stubs: Plugin = {
    name: 'stubs',
    setup(b) {
      for (const [filter] of STUBS) {
        b.onResolve({ filter }, (args) => ({ path: args.path, namespace: 'stub' }));
      }
      b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => {
        const hit = STUBS.find(([filter]) => filter.test(args.path));
        return { contents: hit![1], loader: 'ts', resolveDir: here };
      });
      if (stripFocusReturn) {
        // The dialog primitive's "give focus back to the opener" with its closing half removed.
        b.onLoad({ filter: /\/ui\/dialog\.tsx$/ }, (args) => {
          const source = readFileSync(args.path, 'utf8');
          const from = 'event.preventDefault();\n      target.focus();';
          if (!source.includes(from)) throw new Error('mutation target missing in dialog.tsx');
          return {
            contents: source.replace(from, 'void target;'),
            loader: 'tsx',
            resolveDir: dirname(args.path),
          };
        });
      }
    },
  };
  const out = await build({
    stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
    bundle: true,
    write: false,
    format: 'iife',
    jsx: 'automatic',
    platform: 'browser',
    define: { 'process.env.NODE_ENV': '"production"' },
    loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
    logLevel: 'silent',
    plugins: [stubs],
  });
  return out.outputFiles[0].text;
}

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

/** The marketplace feed the Browse dialog reads: one extension, so the list is not empty. */
const REGISTRY = {
  registry: {
    skills: [],
    extensions: [],
  },
  stale: false,
  fetchedAt: '2026-10-11T00:00:00Z',
};

describe('keyboard focus returns to Add after a dialog closes (V3-R3-2)', () => {
  let browser: Browser | null = null;
  let fixed = '';
  let stripped = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the focus cases.');
      return;
    }
    [fixed, stripped] = await Promise.all([bundle(false), bundle(true)]);
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(script: string): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1100, height: 760 } });
    await tab.setContent(`<!doctype html><html><body><div id="root"></div></body></html>`);
    await tab.evaluate((registry) => {
      (window as unknown as { electron: unknown }).electron = {
        platform: 'darwin',
        on: () => () => {},
        fetchRegistry: async () => registry,
        downloadRegistryAsset: async () => ({ error: 'offline' }),
      };
    }, REGISTRY);
    await tab.addScriptTag({ content: script });
    return tab;
  }

  const addButton = (tab: Page) => tab.getByRole('button', { name: EXTENSIONS_COPY.add });

  /** The menu's items in order, which is how far down the arrow keys have to go. */
  const MENU_ORDER: string[] = [
    EXTENSIONS_COPY.browse,
    EXTENSIONS_COPY.fromFile,
    EXTENSIONS_COPY.custom,
  ];

  /** Focus Add, open its menu with Enter and pick `item` with Enter, as a keyboard user does. */
  async function pickByKeyboard(tab: Page, item: string) {
    const add = addButton(tab);
    await add.waitFor();
    await add.focus();
    await tab.keyboard.press('Enter');
    // A keyboard open puts focus on the first item.
    await tab.waitForFunction(
      (first) => document.activeElement?.textContent?.trim() === first,
      MENU_ORDER[0]
    );
    for (let step = 0; step < MENU_ORDER.indexOf(item); step += 1) {
      await tab.keyboard.press('ArrowDown');
      // Radix moves roving focus in a task of its own.
      await tab.waitForTimeout(60);
    }
    await tab.waitForFunction(
      (label) => document.activeElement?.textContent?.trim() === label,
      item
    );
    await tab.keyboard.press('Enter');
    await tab.getByRole('dialog').waitFor();
    // Radix hands focus to the dialog in an effect after it mounts. A person cannot press Escape
    // before it has, and a case that does is measuring the harness.
    await tab.waitForFunction(() => Boolean(document.activeElement?.closest('[role="dialog"]')));
  }

  const focused = (tab: Page) =>
    tab.evaluate(() => {
      const el = document.activeElement as HTMLElement | null;
      return {
        tag: el?.tagName ?? 'none',
        text: el?.textContent?.trim() ?? '',
        insideDialog: Boolean(el?.closest('[role="dialog"]')),
      };
    });

  async function closeWithEscape(tab: Page) {
    await tab.keyboard.press('Escape');
    await tab.getByRole('dialog').waitFor({ state: 'detached' });
    // Radix returns focus in a task of its own after the content unmounts.
    await tab.waitForTimeout(150);
  }

  const itemsToCheck: Array<[string, string]> = [
    ['Browse marketplace', EXTENSIONS_COPY.browse],
    ['Install from file (.brxt)', EXTENSIONS_COPY.fromFile],
    ['Custom extension', EXTENSIONS_COPY.custom],
  ];

  for (const [name, item] of itemsToCheck) {
    it(`${name}: Escape puts focus back on Add`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(fixed);
      try {
        await pickByKeyboard(tab, item);
        await closeWithEscape(tab);
        const now = await focused(tab);
        expect(now.tag, 'focus must not fall to the page').toBe('BUTTON');
        expect(now.text).toBe(EXTENSIONS_COPY.add);
        expect(now.insideDialog).toBe(false);
      } finally {
        await tab.close();
      }
    });
  }

  // The primitive's rule, kept: a script-focused Add would wear a keyboard ring after a mouse click.
  it('Browse picked with the mouse is not given a keyboard focus back on Escape', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(fixed);
    try {
      await addButton(tab).click();
      await tab.getByRole('menuitem', { name: EXTENSIONS_COPY.browse }).click();
      await tab.getByRole('dialog').waitFor();
      await tab.waitForFunction(() => Boolean(document.activeElement?.closest('[role="dialog"]')));
      await closeWithEscape(tab);
      expect((await focused(tab)).tag).toBe('BODY');
    } finally {
      await tab.close();
    }
  });

  it('Browse picked with the keyboard and closed with the mouse gives nothing back', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(fixed);
    try {
      await pickByKeyboard(tab, EXTENSIONS_COPY.browse);
      // A press on the scrim dismisses an `info` dialog. The page has no stylesheet here, so the
      // dialog's box sits at y 186 to 386 and the window corner is the scrim. The press is held
      // for a few ms, as a hand does: Radix dismisses on pointer-up after a pointer-down.
      await tab.mouse.click(2, 2, { delay: 20 });
      await tab.getByRole('dialog').waitFor({ state: 'detached', timeout: 5000 });
      await tab.waitForTimeout(150);
      expect((await focused(tab)).tag).toBe('BODY');
    } finally {
      await tab.close();
    }
  });

  it('the instrument: with the primitive’s return removed, Browse leaves focus on the page', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(stripped);
    try {
      await pickByKeyboard(tab, EXTENSIONS_COPY.browse);
      await closeWithEscape(tab);
      expect((await focused(tab)).tag).toBe('BODY');
    } finally {
      await tab.close();
    }
  });
});
