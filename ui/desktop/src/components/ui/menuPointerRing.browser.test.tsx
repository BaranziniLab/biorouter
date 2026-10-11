// @vitest-environment node
/**
 * A menu opened with the pointer rings no row (WS-SIDEBAR-2), measured with real input in Chromium
 * under the app's whole stylesheet.
 *
 * `DropdownMenuTrigger` opens on a cancelled `pointerdown`, so the trigger never takes mouse focus
 * and Radix focuses the menu and each row the pointer crosses by script. Chromium reads such a
 * focus as keyboard focus, so a row the person only pointed at matched `:focus-visible` and wore
 * D-15's 2px inset ring. The surface now records how it is driven (`data-menu-input`), and the
 * ring is taken off rows in a `pointer` menu. jsdom has no `:focus-visible` and no cascade, so this
 * runs in Chromium with `styles/main.css` compiled by Tailwind, as `filledPrimitives` does.
 *
 * The first case runs the same steps against the stylesheet WITHOUT the rule and expects the ring:
 * it proves the defect still exists in this Chromium, so the other cases cannot pass vacuously.
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
import { Button } from './button';
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,
  DropdownMenuSub, DropdownMenuSubTrigger, DropdownMenuSubContent,
} from './dropdown-menu';
import { RowContextMenu } from './row-actions';

function App() {
  return (
    <div style={{ padding: 24, display: 'grid', gap: 24, width: 320 }}>
      <button id="before" type="button">Before</button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button id="trigger" variant="ghost" size="sm">Chat actions</Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem data-testid="rename" onSelect={() => {}}>Rename</DropdownMenuItem>
          <DropdownMenuItem data-testid="diverge" onSelect={() => {}}>Diverge</DropdownMenuItem>
          <DropdownMenuSub>
            <DropdownMenuSubTrigger data-testid="move">Move to</DropdownMenuSubTrigger>
            <DropdownMenuSubContent>
              <DropdownMenuItem data-testid="inbox" onSelect={() => {}}>Inbox</DropdownMenuItem>
              <DropdownMenuItem data-testid="archive" onSelect={() => {}}>Archive</DropdownMenuItem>
            </DropdownMenuSubContent>
          </DropdownMenuSub>
        </DropdownMenuContent>
      </DropdownMenu>
      <RowContextMenu
        items={[
          { label: 'View info', onSelect: () => {}, testId: 'ctx-info' },
          { label: 'Delete', onSelect: () => {}, testId: 'ctx-delete' },
        ]}
      >
        <div id="row" tabIndex={0} style={{ height: 40 }}>Row</div>
      </RowContextMenu>
    </div>
  );
}
createRoot(document.getElementById('root')!).render(<App />);
`;

/** The rule under test, as written in main.css. */
const RULE =
  /\[data-menu-input='pointer'\]\s*:is\(\[role='menuitem'\], \[role='menuitemcheckbox'\], \[role='menuitemradio'\]\):focus-visible\s*\{[^}]*\}/;

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

interface Focused {
  testId: string | null;
  role: string | null;
  focusVisible: boolean;
  shadow: string;
  outlineWidth: string;
  surface: string | null;
}

describe('a menu opened with the pointer rings no row (WS-SIDEBAR-2)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the menu pointer ring tests.');
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
    options: { sheet?: string; contrast?: 'more'; forcedColors?: boolean } = {}
  ): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 600, height: 600 } });
    if (options.contrast || options.forcedColors) {
      await tab.emulateMedia({
        contrast: options.contrast ?? 'no-preference',
        forcedColors: options.forcedColors ? 'active' : 'none',
      });
    }
    await tab.setContent(
      `<!doctype html><html><head><style>${options.sheet ?? css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#trigger');
    return tab;
  }

  const focused = (tab: Page) =>
    tab.evaluate((): Focused => {
      const el = document.activeElement as HTMLElement;
      const cs = getComputedStyle(el);
      return {
        testId: el.getAttribute('data-testid'),
        role: el.getAttribute('role'),
        focusVisible: el.matches(':focus-visible'),
        shadow: cs.boxShadow,
        outlineWidth: cs.outlineStyle === 'none' ? '0px' : cs.outlineWidth,
        surface: el.closest('[data-menu-input]')?.getAttribute('data-menu-input') ?? null,
      };
    });

  /** Click the trigger with the mouse, then point at a row. */
  async function pointAt(tab: Page, testId: string) {
    await tab.click('#trigger');
    await tab.waitForSelector('[data-testid="rename"]');
    await tab.hover(`[data-testid="${testId}"]`);
    await tab.waitForTimeout(200);
  }

  const ring = /inset/;

  it('rings a pointed-at row without the rule (the defect this guards is real here)', async (ctx) => {
    if (!browser) return ctx.skip();
    expect(css, 'main.css carries the rule').toMatch(RULE);
    const tab = await open({ sheet: css.replace(RULE, '') });
    try {
      await pointAt(tab, 'diverge');
      const row = await focused(tab);
      expect(row.testId).toBe('diverge');
      expect(row.focusVisible, 'Chromium reads the script focus as keyboard focus').toBe(true);
      expect(row.shadow).toMatch(ring);
    } finally {
      await tab.close();
    }
  });

  it('takes the ring off a row the pointer crosses in a menu the mouse opened', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await pointAt(tab, 'diverge');
      const row = await focused(tab);
      expect(row.testId).toBe('diverge');
      expect(row.surface).toBe('pointer');
      expect(row.shadow).toBe('none');
      // The row keeps its quiet highlight, which is the hover look, not the D-15 focus fill.
      const wash = await tab.evaluate(() => {
        const probe = document.createElement('div');
        probe.style.backgroundColor = 'var(--background-focus)';
        document.body.append(probe);
        const focus = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return {
          focus,
          row: getComputedStyle(document.activeElement as HTMLElement).backgroundColor,
        };
      });
      expect(wash.row).not.toBe(wash.focus);
      expect(wash.row).not.toBe('rgba(0, 0, 0, 0)');
    } finally {
      await tab.close();
    }
  });

  it('puts the ring back on the first key pressed in that menu', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await pointAt(tab, 'rename');
      await tab.keyboard.press('ArrowDown');
      await tab.waitForTimeout(100);
      const row = await focused(tab);
      expect(row.testId).toBe('diverge');
      expect(row.surface).toBe('key');
      expect(row.shadow).toMatch(ring);
    } finally {
      await tab.close();
    }
  });

  it('keeps the ring in a menu the keyboard opened', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.keyboard.press('Tab');
      await tab.keyboard.press('Tab');
      expect((await focused(tab)).role).toBe(null);
      await tab.keyboard.press('Enter');
      await tab.waitForSelector('[data-testid="rename"]');
      await tab.waitForTimeout(100);
      const row = await focused(tab);
      expect(row.testId).toBe('rename');
      expect(row.surface).toBe('key');
      expect(row.shadow).toMatch(ring);
    } finally {
      await tab.close();
    }
  });

  it('rings no row in a submenu the pointer opened, and rings one the keyboard opened', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await pointAt(tab, 'move');
      await tab.waitForSelector('[data-testid="inbox"]');
      await tab.hover('[data-testid="archive"]');
      await tab.waitForTimeout(200);
      const pointed = await focused(tab);
      expect(pointed.testId).toBe('archive');
      expect(pointed.surface).toBe('pointer');
      expect(pointed.shadow).toBe('none');
    } finally {
      await tab.close();
    }
    const keyed = await open();
    try {
      await pointAt(keyed, 'move');
      await keyed.keyboard.press('ArrowRight');
      await keyed.waitForSelector('[data-testid="inbox"]');
      await keyed.waitForTimeout(100);
      const row = await focused(keyed);
      expect(row.testId).toBe('inbox');
      expect(row.surface).toBe('key');
      expect(row.shadow).toMatch(ring);
    } finally {
      await keyed.close();
    }
  });

  it('leaves the right-click context menu as it was: no ring under the pointer', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.click('#row', { button: 'right' });
      await tab.waitForSelector('[data-testid="ctx-info"]');
      await tab.hover('[data-testid="ctx-delete"]');
      await tab.waitForTimeout(200);
      const row = await focused(tab);
      expect(row.testId).toBe('ctx-delete');
      expect(row.shadow).not.toMatch(ring);
    } finally {
      await tab.close();
    }
  });

  it('keeps the outline on a pointed-at row for a person who asked for more contrast', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const options of [{ contrast: 'more' as const }, { forcedColors: true }]) {
      const tab = await open(options);
      try {
        await pointAt(tab, 'diverge');
        const row = await focused(tab);
        expect(row.testId).toBe('diverge');
        expect(row.outlineWidth, JSON.stringify(options)).toBe('2px');
      } finally {
        await tab.close();
      }
    }
  });
});
