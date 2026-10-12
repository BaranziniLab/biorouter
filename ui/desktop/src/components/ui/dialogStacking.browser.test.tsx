// @vitest-environment node
/**
 * A dialog opened from a dialog dims the one beneath it (vision round 1, V4-08).
 *
 * Stacking is a question jsdom cannot answer, so the real `Dialog` primitive is bundled with
 * esbuild, rendered by React in Chromium with the overlay's and the surface's rules (the
 * authored `@ws WS-PRIMITIVES` blocks and `.biorouter-modal-surface` from `styles/main.css`, plus
 * a layered copy of the utilities the components write), and the page is hit-tested: inside the
 * first dialog but outside the second, the second dialog's scrim must be on top; inside the
 * second, the second's surface; and a lone dialog's surface stays above its own scrim.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

/** Every `@ws WS-PRIMITIVES` block of main.css, in file order. */
function primitiveBlocks(): string {
  const re =
    /\/\* @ws WS-PRIMITIVES begin: ([^*]+?) \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: \1 \*\//g;
  const out = Array.from(MAIN_CSS.matchAll(re), (match) => match[2]);
  if (out.length === 0) throw new Error('main.css has no @ws WS-PRIMITIVES block');
  return out.join('\n');
}

/** The surface's unlayered rule, which carries its `z-index: var(--z-modal)` floor. */
function surfaceRule(): string {
  const match = MAIN_CSS.match(/\n\.biorouter-modal-surface \{[\s\S]*?\n\}/);
  if (!match) throw new Error('main.css has no .biorouter-modal-surface rule');
  return match[0];
}

/** The z tokens, read from main.css. */
function zTokens(): string {
  const lines = ['--z-overlay', '--z-modal'].map((name) => {
    const match = MAIN_CSS.match(new RegExp(`\\n\\s*${name}:\\s*([^;]+);`));
    if (!match) throw new Error(`main.css does not declare ${name}`);
    return `${name}: ${match[1].trim()};`;
  });
  return `:root { ${lines.join(' ')} }`;
}

/** The utilities the overlay and surface write, in Tailwind's layer. */
const UTILITIES = `
@layer utilities {
  .fixed { position: fixed; }
  .inset-0 { inset: 0; }
  .z-\\[var\\(--z-overlay\\)\\] { z-index: var(--z-overlay); }
  .z-\\[var\\(--z-modal\\)\\] { z-index: var(--z-modal); }
}
body { margin: 0; }
/* Radix sets pointer-events: none on every layer below the top one, and hit testing skips such
   elements, so without this a covered dialog and an uncovered one hit-test alike (the first
   draft of this test passed against the old stacking). Forced back on, the hit test reads the
   paint order. */
body, [data-slot] { pointer-events: auto !important; }
:root { --scrim: rgb(0 0 0 / 0.3); --background-default: #fff; --border-subtle: #ccc;
  --radius-container: 12px; --shadow-modal: none; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Dialog, DialogContent, DialogTitle, DialogDescription } from './dialog';

function App({ nested }: { nested: boolean }) {
  return (
    <Dialog open>
      <DialogContent
        data-testid="outer"
        style={{ position: 'fixed', top: 40, left: 40, width: 600, height: 400, transform: 'none' }}
      >
        <DialogTitle>Outer</DialogTitle>
        <DialogDescription>Tool permissions</DialogDescription>
        {nested ? (
          <Dialog open>
            <DialogContent
              data-testid="inner"
              style={{ position: 'fixed', top: 120, left: 120, width: 200, height: 120, transform: 'none' }}
            >
              <DialogTitle>Inner</DialogTitle>
              <DialogDescription>Developer</DialogDescription>
            </DialogContent>
          </Dialog>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

const nested = (window as unknown as { nestedDialog?: boolean }).nestedDialog === true;
createRoot(document.getElementById('root')!).render(<App nested={nested} />);
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

describe('a dialog opened from a dialog (V4-08)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the dialog stacking tests.');
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
    css = UTILITIES + zTokens() + surfaceRule() + primitiveBlocks();
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(nested: boolean): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 600 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.evaluate((value) => {
      (window as unknown as { nestedDialog?: boolean }).nestedDialog = value;
    }, nested);
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector(nested ? '[data-testid="inner"]' : '[data-testid="outer"]');
    return tab;
  }

  /** What is on top at (x, y): the test id of the dialog, or which scrim. */
  const topAt = (tab: Page, x: number, y: number) =>
    tab.evaluate(
      ([px, py]) => {
        const hit = document.elementFromPoint(px, py);
        if (!hit) return 'nothing';
        const surface = hit.closest('[data-testid]');
        if (surface) return surface.getAttribute('data-testid');
        if (hit.matches('[data-slot="dialog-overlay"]')) {
          const scrims = [...document.querySelectorAll('[data-slot="dialog-overlay"]')];
          return `scrim ${scrims.indexOf(hit) + 1} of ${scrims.length}`;
        }
        return hit.tagName.toLowerCase();
      },
      [x, y]
    );

  it('keeps a lone dialog above its own scrim', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(false);
    try {
      expect(await topAt(tab, 300, 300)).toBe('outer');
      expect(await topAt(tab, 700, 550)).toBe('scrim 1 of 1');
    } finally {
      await tab.close();
    }
  });

  it('puts the second dialog’s scrim over the first dialog, and the second dialog over both', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(true);
    try {
      // Inside the first dialog, outside the second: the second's scrim dims it.
      expect(await topAt(tab, 500, 380)).toBe('scrim 2 of 2');
      // Inside the second dialog.
      expect(await topAt(tab, 200, 160)).toBe('inner');
      // Outside both.
      expect(await topAt(tab, 700, 550)).toBe('scrim 2 of 2');
    } finally {
      await tab.close();
    }
  });
});
