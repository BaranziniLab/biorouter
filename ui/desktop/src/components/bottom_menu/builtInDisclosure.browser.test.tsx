// @vitest-environment node
/**
 * The built-in section of a picker list (`PickerDisclosure`, owner 2026-11-13)
 * measured in a real layout engine.
 *
 * jsdom lays nothing out and never reads `pickers.css`, and the trigger's look is
 * authored CSS only (a newly written Tailwind utility can silently fail to
 * generate under `BIOROUTER_NO_HMR`). So the real `PickerPanel`, `PickerRow` and
 * `PickerDisclosure` are bundled with esbuild, rendered in Chromium and styled by
 * `pickers.css` and the primitive blocks of `main.css`, as
 * `pickersGeometry.browser.test.tsx` does for the rest of the list.
 *
 * What is held: the trigger is a 32px row whose label starts on the rows' text
 * edge and whose chevron ends on the switches' trailing edge; the rows it opens
 * are ordinary 32px rows inside the same scrolling list; and a Tab-focused
 * trigger takes D-15's focus surface although it paints its own ground.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const PICKERS_CSS = readFileSync(resolve(here, 'pickers.css'), 'utf8');

const TOKEN_NAMES = [
  '--control-sm',
  '--control-md',
  '--radius-full',
  '--radius-element',
  '--radius-inner',
  '--icon-chip',
  '--text-supporting',
  '--text-supporting--line-height',
  '--text-secondary',
  '--text-secondary--line-height',
  '--dur-fast-min',
  '--dur-fast',
  '--dur-fast-max',
  '--ease-out',
  '--ease-spring',
];

function readTokens(): string {
  const lines = TOKEN_NAMES.map((name) => {
    const match = MAIN_CSS.match(new RegExp(`\\n\\s*${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
    if (!match) throw new Error(`main.css does not declare ${name}`);
    return `  ${name}: ${match[1].trim()};`;
  });
  return `:root {\n${lines.join('\n')}\n}`;
}

function primitiveBlocks(): string {
  const out: string[] = [];
  const re =
    /\/\* @ws WS-PRIMITIVES begin: ([^*]+?) \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: \1 \*\//g;
  for (const match of MAIN_CSS.matchAll(re)) out.push(match[2]);
  if (out.length === 0) throw new Error('main.css has no @ws WS-PRIMITIVES block');
  return out.join('\n');
}

const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
button, input { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
svg { display: block; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
:root { --control-track-off: #ebebe8; --background-accent: #b85a32; --border-control: #86847f;
  --background-default: #fff; --text-on-accent: #fff; --text-muted: #666; --text-default: #111;
  --text-subtle: #777; --background-muted: #f2f2f0; --border-focus: #555; --background-focus: #ddd;
  --shadow-raised: 0 1px 2px rgb(0 0 0 / 0.1); --inset-hairline: inset 0 0 0 1px #ddd;
  --overlay-hover: rgb(0 0 0 / 0.05); --overlay-pressed: rgb(0 0 0 / 0.1); --border-subtle: #ccc; }
`;

/** `CommandList`'s `p-1`, which puts the rows' text 16px in. */
const UTILITIES = `@layer utilities { .p-1 { padding: 4px; } }`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { PickerDisclosure, PickerPanel, PickerRow } from './PickerPanel';

function App() {
  const [query, setQuery] = React.useState('');
  const [open, setOpen] = React.useState(true);
  return (
    <div style={{ padding: 16 }}>
      <div className="br-picker-popover" id="popover">
        <PickerPanel
          listLabel="Extensions"
          searchPlaceholder="Search extensions…"
          query={query}
          onQueryChange={setQuery}
          scopeLabel="This chat"
          scopeHelp="Changes apply to this chat only."
          emptyText="No extensions found"
          trailing={
            <PickerDisclosure label="Built-in capabilities" summary="12 on" open={open} onOpenChange={setOpen}>
              <PickerRow name="Developer" checked onToggle={() => {}} data-testid="builtin-row" />
              <PickerRow name="Chat Recall" checked={false} onToggle={() => {}} />
            </PickerDisclosure>
          }
        >
          <PickerRow name="spoke" checked onToggle={() => {}} data-testid="own-row" />
        </PickerPanel>
      </div>
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

const EPSILON = 0.5;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

describe('the built-in section of a picker list in a real layout engine', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the built-in disclosure geometry tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      loader: { '.css': 'empty' },
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css = PREFLIGHT + UTILITIES + readTokens() + primitiveBlocks() + PICKERS_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 600, height: 700 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('.br-picker-disclosure__trigger');
    await tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );
    return tab;
  }

  it('draws the trigger as a 32px row on the rows’ text and switch edges', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const box = (el: Element) => el.getBoundingClientRect();
        const trigger = document.querySelector('.br-picker-disclosure__trigger')!;
        const label = document.querySelector('.br-picker-disclosure__label')!;
        const chevron = document.querySelector('.br-picker-disclosure__chevron')!;
        const own = document.querySelector('[data-testid="own-row"]')!;
        const builtIn = document.querySelector('[data-testid="builtin-row"]')!;
        const list = document.querySelector('[role="listbox"]')!;
        return {
          trigger: box(trigger).height,
          labelLeft: box(label).left,
          ownNameLeft: box(own.querySelector('.br-picker-row__name')!).left,
          chevronRight: box(chevron).right,
          ownSwitchRight: box(own.querySelector('.br-picker-row__switch')!).right,
          chevronSize: box(chevron).width,
          builtInHeight: box(builtIn).height,
          builtInNameLeft: box(builtIn.querySelector('.br-picker-row__name')!).left,
          // The rows the trigger opens are inside the one scrolling list.
          builtInInList: list.contains(builtIn),
          chevronTransform: getComputedStyle(chevron).transform,
          labelColor: getComputedStyle(trigger).color,
        };
      });
      near(m.trigger, 32, 'trigger height');
      near(m.labelLeft, m.ownNameLeft, 'label on the rows’ text edge');
      near(m.chevronRight, m.ownSwitchRight, 'chevron on the switches’ trailing edge');
      near(m.chevronSize, 14, 'chevron size');
      near(m.builtInHeight, 32, 'a built-in row');
      near(m.builtInNameLeft, m.ownNameLeft, 'a built-in row’s name');
      expect(m.builtInInList).toBe(true);
      // Open: turned a quarter (matrix(0, 1, -1, 0, …) is rotate(90deg)).
      expect(m.chevronTransform).toMatch(/^matrix\(\s*0(\.0+)?(e-\d+)?,\s*1,/);
      // Muted at rest: the trigger is a section head, not one of the rows.
      expect(m.labelColor).toBe('rgb(102, 102, 102)');
    } finally {
      await tab.close();
    }
  });

  it('gives a Tab-focused trigger the focus surface, though it paints its own ground', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.focus('[role="combobox"]');
      // Field, InfoTip, then the trigger.
      let focused = '';
      for (let i = 0; i < 4 && !focused.includes('br-picker-disclosure__trigger'); i += 1) {
        await tab.keyboard.press('Tab');
        focused = await tab.evaluate(() => document.activeElement?.className ?? '');
      }
      expect(focused).toContain('br-picker-disclosure__trigger');
      // Past the ground's transition (--dur-fast-min).
      await tab.waitForTimeout(300);
      const style = await tab.evaluate(() => {
        const s = getComputedStyle(document.activeElement!);
        return { background: s.backgroundColor, shadow: s.boxShadow };
      });
      expect(style.background).toBe('rgb(221, 221, 221)');
      expect(style.shadow).toContain('inset');

      // Space closes it, and the rows go with it.
      await tab.keyboard.press('Space');
      await tab.waitForFunction(() => !document.querySelector('[data-testid="builtin-row"]'));
    } finally {
      await tab.close();
    }
  });
});
