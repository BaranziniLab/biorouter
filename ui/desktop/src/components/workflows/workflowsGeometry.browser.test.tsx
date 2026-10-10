// @vitest-environment node
/**
 * What jsdom cannot see about Workflows, in a real layout engine (spec 3.10 and 5.7).
 *
 * jsdom lays nothing out and paints nothing, so a component test can only pin class names. Two
 * defects of this surface were invisible to every test until a real browser measured them, and
 * both are cascade or stacking facts rather than markup facts:
 *
 * 1. **A row's actions take the click, not its meta.** `RowActions` stacks a `meta` (the relative
 *    time) and the actions cluster in one grid cell, and on hover the meta fades to
 *    `opacity: 0`. That makes it a stacking context, which paints ABOVE the opacity-1 cluster in
 *    normal flow, so the invisible time covered the right half of the `⋯` button: a click there
 *    opened the workflow editor (the row's own handler took it) or, on a Built apps row, did
 *    nothing. `document.elementFromPoint` is the instrument; `pointer-events: none` on
 *    `.br-row-actions-meta` (WS-PRIMITIVES, from `requests/WS-PRIMITIVES/WS-VIEWS-A-3.md`) is the rule.
 * 2. **The editors' text areas are tall.** `.br-textarea` (main.css) is unlayered and sets its own
 *    `min-block-size` and `resize`, so a Tailwind `min-h-[400px]` on the same element loses to it:
 *    the Instructions and Response schema editors were 74px tall, three rows, in an 800px dialog.
 *    `workflows.css` carries the authored rule that wins.
 *
 * The real `RowActions` is bundled with esbuild and rendered by React in Chromium, styled by the
 * authored `@ws WS-PRIMITIVES` blocks of `styles/main.css` and by this workstream's own CSS.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const WORKFLOWS_CSS = readFileSync(resolve(here, 'workflows.css'), 'utf8');

const TOKEN_NAMES = ['--dur-fast-min', '--ease-out'];

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

/** Tailwind's preflight, the parts the layout depends on, and the buttons' own 32px box. */
const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
button { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
svg { display: block; width: 16px; height: 16px; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
.biorouter-list-row { display: flex; align-items: center; gap: 12px; width: 600px; padding: 8px 12px; }
button[aria-label] { display: inline-flex; width: 32px; height: 32px; align-items: center; justify-content: center; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Play } from '../icons/app-icons';
import { IconAction, RowActions } from '../ui/row-actions';

function App() {
  return (
    <div className="biorouter-list-row" id="row">
      <div style={{ flex: 1 }}>A row</div>
      <RowActions
        primary={<IconAction icon={Play} label="Run" onSelect={() => {}} />}
        menu={[{ label: 'Edit', onSelect: () => {} }]}
        menuLabel="More actions"
        meta={<time className="tabular-nums">35m</time>}
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

describe('Workflows in a real layout engine (spec 3.10, 5.7)', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the row actions hit tests.'
      );
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
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(withRule: boolean): Promise<Page> {
    const tab = await browser!.newPage({
      viewport: { width: 800, height: 200 },
      reducedMotion: 'reduce',
    });
    const primitives = primitiveBlocks();
    // "Without the rule" is the primitive's own CSS with that one declaration removed, so the
    // instrument is proved against the real stylesheet and not a hand-made copy of it.
    const css =
      PREFLIGHT +
      readTokens() +
      (withRule
        ? primitives
        : primitives.replace(/(\.br-row-actions-meta \{[^}]*?)pointer-events:\s*none;/, '$1'));
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#row button[aria-label="More actions"]');
    return tab;
  }

  /** Whether the element at a point of the `⋯` button belongs to that button, hovered or not. */
  async function hits(tab: Page, hover: boolean) {
    if (hover) {
      const row = (await tab.locator('#row').boundingBox())!;
      await tab.mouse.move(row.x + 40, row.y + row.height / 2);
      await tab.waitForTimeout(150);
    }
    return tab.evaluate(() => {
      const more = document.querySelector('#row button[aria-label="More actions"]')!;
      const run = document.querySelector('#row button[aria-label="Run"]')!;
      const b = more.getBoundingClientRect();
      const at = (x: number, y: number) => {
        const el = document.elementFromPoint(x, y);
        return el ? more.contains(el) : false;
      };
      const r = run.getBoundingClientRect();
      const time = document.querySelector('#row time')!.getBoundingClientRect();
      return {
        centre: at(b.left + b.width / 2, b.top + b.height / 2),
        rightEdge: at(b.right - 3, b.top + b.height / 2),
        leftEdge: at(b.left + 3, b.top + b.height / 2),
        run: (() => {
          const el = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
          return el ? run.contains(el) : false;
        })(),
        // The geometry that makes the bug possible: the time sits inside the button's box.
        timeOverlapsButton: time.left < b.right && time.right > b.left,
      };
    });
  }

  it('lets a click on the `⋯` button land on the button once the row is hovered', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(true);
    try {
      const rest = await hits(tab, false);
      expect(rest.centre, 'centre at rest').toBe(true);
      const hovered = await hits(tab, true);
      expect(hovered.timeOverlapsButton, 'the fixture must overlap, or it proves nothing').toBe(
        true
      );
      expect(hovered.centre, 'centre when hovered').toBe(true);
      expect(hovered.rightEdge, 'right edge when hovered').toBe(true);
      expect(hovered.leftEdge, 'left edge when hovered').toBe(true);
      expect(hovered.run, 'Run when hovered').toBe(true);
    } finally {
      await tab.close();
    }
  });

  it('proves the instrument: without the rule the invisible time takes the click', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(false);
    try {
      const hovered = await hits(tab, true);
      expect(hovered.timeOverlapsButton).toBe(true);
      // If stripping the declaration changes nothing, the rule is not what holds the line any
      // more (the geometry changed), and this case has nothing left to prove.
      expect(hovered.centre, 'the invisible meta must take the click without the rule').toBe(false);
    } finally {
      await tab.close();
    }
  });

  it('keeps the editors’ text areas tall against the primitive’s own min-block-size', async (ctx) => {
    if (!browser) return ctx.skip();
    // The `.br-textarea` rule exactly as main.css declares it, here injected UNLAYERED so it
    // reproduces what it used to do to a layered Tailwind utility (it is now in
    // `@layer components`, where a caller's utility wins; WS-PRIMITIVES layered it).
    const textareaRule = MAIN_CSS.match(/\n\s*\.br-textarea \{[^}]*\}/)?.[0];
    if (!textareaRule) throw new Error('main.css has no `.br-textarea` rule');
    const measure = async (css: string) => {
      const tab = await browser!.newPage({ viewport: { width: 900, height: 700 } });
      try {
        await tab.setContent(
          `<!doctype html><html><head><style>*{box-sizing:border-box}
            :root{--text-body--line-height:20px}
            textarea{display:block;width:700px;padding:6px 8px;font:14px/20px Arial,sans-serif}
            ${textareaRule}${css}</style></head><body>
            <textarea class="br-field br-textarea br-workflow-editor-field" rows="2"></textarea>
          </body></html>`
        );
        return await tab.evaluate(() => {
          const el = document.querySelector('textarea')!;
          return { height: el.getBoundingClientRect().height, resize: getComputedStyle(el).resize };
        });
      } finally {
        await tab.close();
      }
    };
    const withRule = await measure(WORKFLOWS_CSS);
    expect(withRule.height, 'editor text area height').toBeGreaterThanOrEqual(400);
    expect(withRule.resize).toBe('none');
    // The instrument: with only the primitive's rule the same element is the 74px it was.
    const without = await measure('');
    expect(without.height, 'without the authored rule').toBeLessThan(100);
  });
});
