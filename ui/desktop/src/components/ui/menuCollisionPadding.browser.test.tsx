// @vitest-environment node
/**
 * Every menu keeps 8px from the window's edge (vision V4-R6-1), measured in Chromium with real
 * pointer input under the app's whole stylesheet.
 *
 * A row menu right-clicked near the bottom of the sidebar ended on the window's last pixel: its
 * bottom hairline and corners sat on the edge and its shadow was cut off, because
 * `ContextMenuContent` and `DropdownMenuContent` passed no `collisionPadding` and Radix allows 0px.
 * Both contents (and the dropdown's submenu) now default to `MENU_COLLISION_PADDING`.
 *
 * jsdom has no layout, so this runs in Chromium. Each case runs twice: once with the primitives as
 * shipped, and once with `collisionPadding={0}` passed explicitly (the old default), which must put
 * the menu on the edge, so the shipped cases cannot pass vacuously.
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

/** Set to a directory to keep a picture of the row menu with and without the gap. */
const SHOT_DIR = process.env.MENU_SHOT_DIR;

const WIDTH = 800;
const HEIGHT = 600;
const GAP = 8;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Button } from './button';
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,
  DropdownMenuSub, DropdownMenuSubTrigger, DropdownMenuSubContent,
} from './dropdown-menu';
import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from './context-menu';
import { RowContextMenuItems } from './row-actions';

// undefined: the primitives' own default. A number: passed explicitly (the instrument).
const override = (window as unknown as { __padding?: number }).__padding;
const pad = override === undefined ? {} : { collisionPadding: override };

const ROW_ITEMS = ['Rename', 'Open in new tab', 'Open in new window', 'Pin', 'Move to folder', 'Copy chat ID', 'Export', 'Delete chat']
  .map((label) => ({ label, onSelect: () => {}, testId: 'row-' + label }));

function App() {
  return (
    <>
      {/* The sidebar: a row on its last line, as in the finding (RowContextMenu's own parts). */}
      <div style={{ position: 'fixed', left: 0, top: 0, bottom: 0, width: 240 }}>
        <ContextMenu>
          <ContextMenuTrigger asChild>
            <div id="row" tabIndex={0} style={{ position: 'absolute', left: 8, right: 8, bottom: 4, height: 32 }}>
              Last chat
            </div>
          </ContextMenuTrigger>
          <ContextMenuContent {...pad}>
            <RowContextMenuItems items={ROW_ITEMS} />
          </ContextMenuContent>
        </ContextMenu>
      </div>
      {/* A dropdown flush on the window's left edge, with more rows than the window holds. */}
      <div style={{ position: 'fixed', left: 0, top: 280 }}>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button id="long-trigger" variant="ghost" size="sm">Long</Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" {...pad}>
            {Array.from({ length: 30 }, (_, i) => (
              <DropdownMenuItem key={i} data-testid={'long-' + i} onSelect={() => {}}>Row {i}</DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      {/* A dropdown near the bottom whose last row opens a submenu downward. */}
      <div style={{ position: 'fixed', left: 300, bottom: 4 }}>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button id="sub-trigger-host" variant="ghost" size="sm">More</Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" {...pad}>
            <DropdownMenuItem onSelect={() => {}}>Rename</DropdownMenuItem>
            <DropdownMenuItem onSelect={() => {}}>Pin</DropdownMenuItem>
            <DropdownMenuSub>
              <DropdownMenuSubTrigger data-testid="move">Move to</DropdownMenuSubTrigger>
              <DropdownMenuSubContent {...pad}>
                {Array.from({ length: 10 }, (_, i) => (
                  <DropdownMenuItem key={i} data-testid={'folder-' + i} onSelect={() => {}}>Folder {i}</DropdownMenuItem>
                ))}
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </>
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

interface Box {
  top: number;
  right: number;
  bottom: number;
  left: number;
  innerWidth: number;
  innerHeight: number;
}

describe('menus keep 8px from the window edge (V4-R6-1)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the menu collision padding tests.');
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

  async function open(padding?: number): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: WIDTH, height: HEIGHT } });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    if (padding !== undefined) {
      await tab.evaluate((value) => {
        (window as unknown as { __padding?: number }).__padding = value;
      }, padding);
    }
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#row');
    return tab;
  }

  /** The settled box of the menu surface a selector names, once its entrance has finished. */
  async function settledBox(tab: Page, selector: string): Promise<Box> {
    await tab.waitForSelector(selector);
    await tab.waitForFunction(() =>
      document.getAnimations().every((animation) => animation.playState !== 'running')
    );
    await tab.waitForTimeout(50);
    return tab.evaluate((sel) => {
      const rect = (document.querySelector(sel) as HTMLElement).getBoundingClientRect();
      return {
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
        left: rect.left,
        innerWidth: window.innerWidth,
        innerHeight: window.innerHeight,
      };
    }, selector);
  }

  /** Right-click the sidebar's last row, near its bottom-left corner, as in the finding. */
  async function rowMenu(tab: Page): Promise<Box> {
    await tab.mouse.click(24, HEIGHT - 12, { button: 'right' });
    return settledBox(tab, '[data-slot="context-menu-content"]');
  }

  async function longMenu(tab: Page): Promise<Box> {
    await tab.click('#long-trigger');
    return settledBox(tab, '[data-slot="dropdown-menu-content"]');
  }

  async function subMenu(tab: Page): Promise<Box> {
    await tab.click('#sub-trigger-host');
    await tab.waitForSelector('[data-testid="move"]');
    await tab.hover('[data-testid="move"]');
    return settledBox(tab, '[data-slot="dropdown-menu-sub-content"]');
  }

  it('puts the menus on the edge with no padding (the defect this guards is real here)', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const [name, openMenu, edge] of [
      ['row menu', rowMenu, 'bottom'],
      ['long dropdown', longMenu, 'left'],
      ['submenu', subMenu, 'bottom'],
    ] as const) {
      const tab = await open(0);
      try {
        const box = await openMenu(tab);
        if (SHOT_DIR && name === 'row menu')
          await tab.screenshot({ path: resolve(SHOT_DIR, 'row-menu-no-padding.png') });
        if (edge === 'bottom') expect(box.bottom, name).toBeCloseTo(box.innerHeight, 0);
        else expect(box.left, name).toBeCloseTo(0, 0);
      } finally {
        await tab.close();
      }
    }
  });

  it('keeps a row menu right-clicked at the bottom-left 8px inside the window', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const box = await rowMenu(tab);
      if (SHOT_DIR) await tab.screenshot({ path: resolve(SHOT_DIR, 'row-menu.png') });
      expect(box.bottom).toBeLessThanOrEqual(box.innerHeight - GAP + 0.5);
      // It moved up only as far as the gap asks, not off to some other corner.
      expect(box.bottom).toBeGreaterThanOrEqual(box.innerHeight - GAP - 1);
      expect(box.left).toBeGreaterThanOrEqual(GAP - 0.5);
      expect(box.top).toBeGreaterThanOrEqual(GAP - 0.5);
    } finally {
      await tab.close();
    }
  });

  it('keeps a dropdown taller than the window 8px from every edge', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const box = await longMenu(tab);
      expect(box.left).toBeGreaterThanOrEqual(GAP - 0.5);
      expect(box.top).toBeGreaterThanOrEqual(GAP - 0.5);
      expect(box.bottom).toBeLessThanOrEqual(box.innerHeight - GAP + 0.5);
    } finally {
      await tab.close();
    }
  });

  it('keeps a submenu that opens downward near the bottom 8px inside the window', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const box = await subMenu(tab);
      expect(box.bottom).toBeLessThanOrEqual(box.innerHeight - GAP + 0.5);
      expect(box.top).toBeGreaterThanOrEqual(GAP - 0.5);
      expect(box.right).toBeLessThanOrEqual(box.innerWidth - GAP + 0.5);
    } finally {
      await tab.close();
    }
  });

  it('lets a caller still choose its own padding', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(24);
    try {
      const box = await longMenu(tab);
      expect(box.left).toBeGreaterThanOrEqual(24 - 0.5);
      expect(box.bottom).toBeLessThanOrEqual(box.innerHeight - 24 + 0.5);
    } finally {
      await tab.close();
    }
  });
});
