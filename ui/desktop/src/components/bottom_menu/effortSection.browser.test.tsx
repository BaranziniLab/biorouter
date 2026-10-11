// @vitest-environment node
/**
 * R-26 (owner message 13): the model menu's Effort section, measured in Chromium inside a real
 * Radix `DropdownMenu`, with the real `Slider`, `InfoTip` and effort store.
 *
 * jsdom can test the wiring (ModelsBottomBar.effort.test.tsx) but not three things that only a
 * layout engine and real input can: that the head row and the slider line up on the menu's 12px
 * text edge, that a real press and drag inside an open MENU (which steals focus on pointer moves
 * and closes on outside presses) sets the stop and leaves the menu open, and that the arrow keys
 * really carry focus between the menu's rows and the thumb. `pickers.css` and the
 * `@ws WS-PRIMITIVES` blocks of `main.css` supply the styles; Tailwind does not run, so the
 * menu's own width and padding are set inline at the recipe's values (w-72, p-1).
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
button { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
svg { display: block; width: 14px; height: 14px; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
:root { --control-track-off: #ebebe8; --background-accent: #b85a32; --border-control: #86847f;
  --background-default: #fff; --text-on-accent: #fff; --text-muted: #666; --text-default: #111;
  --background-muted: #f2f2f0; --border-focus: #555; --background-focus: #ddd;
  --border-subtle: #ccc; --overlay-hover: rgb(0 0 0 / 0.05); }
#menu { width: 288px; padding: 4px; background: #fff; box-shadow: 0 0 0 1px #ccc; }
[role='menuitem'] { display: flex; align-items: center; height: 32px; padding: 0 12px; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { EffortSection, moveFocusAcrossEffort } from './EffortSection';
import {
  getReasoningEffort,
  subscribeToReasoningEffort,
} from '../../store/reasoningEffort';

const SCOPE = 'session:geometry';

function App() {
  const effort = React.useSyncExternalStore(
    (listener) => subscribeToReasoningEffort(SCOPE, listener),
    () => getReasoningEffort(SCOPE)
  );
  const thumb = React.useRef<HTMLSpanElement>(null);
  (window as unknown as { __effort: () => string }).__effort = () => getReasoningEffort(SCOPE);
  return (
    <div style={{ position: 'fixed', left: 40, bottom: 40 }}>
      <DropdownMenu>
        <DropdownMenuTrigger id="chip">Model</DropdownMenuTrigger>
        <DropdownMenuContent
          id="menu"
          side="top"
          align="start"
          onKeyDownCapture={(event) => moveFocusAcrossEffort(event, thumb.current)}
        >
          <EffortSection scope={SCOPE} effort={effort} thumbRef={thumb} />
          <DropdownMenuSeparator />
          <DropdownMenuItem id="change">Change model…</DropdownMenuItem>
          <DropdownMenuItem id="lead">Lead and worker…</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
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

describe('the model menu’s Effort slider in a real layout engine (R-26)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the R-26 tests.');
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
    css = PREFLIGHT + readTokens() + primitiveBlocks() + PICKERS_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 600 } });
    await tab.emulateMedia({ reducedMotion: 'reduce' });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.click('#chip');
    await tab.waitForSelector('[data-testid="effort-slider"]');
    await tab.waitForTimeout(250);
    return tab;
  }

  const effort = (tab: Page) =>
    tab.evaluate(() => (window as unknown as { __effort: () => string }).__effort());

  it('lines the head row and the slider up on the menu’s 12px text edge', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const box = (selector: string) => document.querySelector(selector)!.getBoundingClientRect();
        const section = box('[data-testid="effort-section"]');
        const label = document.querySelector('.br-effort__label')!;
        const value = document.querySelector('[data-testid="effort-value"]')!;
        return {
          sectionLeft: section.left,
          sectionRight: section.right,
          labelLeft: label.getBoundingClientRect().left,
          labelSize: getComputedStyle(label).fontSize,
          labelWeight: getComputedStyle(label).fontWeight,
          valueRight: value.getBoundingClientRect().right,
          valueText: value.textContent,
          valueSize: getComputedStyle(value).fontSize,
          slider: box('[data-testid="effort-slider"]'),
          thumb: box('[role="slider"]'),
          head: box('.br-effort__head'),
        };
      });
      near(m.labelLeft - m.sectionLeft, 12, '"Effort" on the 12px text edge');
      near(m.sectionRight - m.valueRight, 12, 'current stop on the 12px trailing edge');
      expect(m.labelSize).toBe('12px');
      expect(m.labelWeight).toBe('500');
      expect(m.valueSize).toBe('12px');
      expect(m.valueText).toBe('Normal');
      // The slider fills the section inside its padding, 24px tall, under the head row.
      near(m.slider.left - m.sectionLeft, 12, 'slider left');
      near(m.sectionRight - m.slider.right, 12, 'slider right');
      near(m.slider.height, 24, 'slider height');
      expect(m.slider.top).toBeGreaterThanOrEqual(m.head.bottom);
      // Normal is the middle stop.
      near(
        m.thumb.left + m.thumb.width / 2,
        m.slider.left + m.slider.width / 2,
        'Normal thumb centred'
      );
    } finally {
      await tab.close();
    }
  });

  it('sets the stop a press or a drag ends on, and leaves the menu open', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const slider = (await tab.locator('[data-testid="effort-slider"]').boundingBox())!;
      const y = slider.y + slider.height / 2;
      // A press near the right end.
      await tab.mouse.click(slider.x + slider.width - 4, y);
      expect(await effort(tab)).toBe('deep');
      // A drag back to the left end, released there.
      await tab.mouse.move(slider.x + slider.width - 8, y);
      await tab.mouse.down();
      await tab.mouse.move(slider.x + slider.width / 2, y, { steps: 4 });
      await tab.mouse.move(slider.x + 2, y, { steps: 4 });
      await tab.mouse.up();
      expect(await effort(tab)).toBe('quick');
      await tab.waitForTimeout(250);
      const m = await tab.evaluate(() => ({
        open: document.querySelector('[data-testid="effort-section"]') !== null,
        value: document.querySelector('[data-testid="effort-value"]')?.textContent,
        valuetext: document.querySelector('[role="slider"]')?.getAttribute('aria-valuetext'),
      }));
      expect(m).toEqual({ open: true, value: 'Quick', valuetext: 'Quick' });
    } finally {
      await tab.close();
    }
  });

  it('carries focus between the rows and the thumb with the arrow keys', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const focused = () =>
        tab.evaluate(() => {
          const el = document.activeElement;
          return el?.getAttribute('role') === 'slider' ? 'thumb' : (el?.id ?? null);
        });
      await tab.focus('#change');
      await tab.keyboard.press('ArrowUp');
      expect(await focused()).toBe('thumb');
      await tab.keyboard.press('ArrowRight');
      expect(await effort(tab)).toBe('deep');
      await tab.keyboard.press('ArrowLeft');
      await tab.keyboard.press('ArrowLeft');
      expect(await effort(tab)).toBe('quick');
      await tab.keyboard.press('ArrowDown');
      expect(await focused()).toBe('change');
      // Row to row is Radix's own, which moves focus on a timer.
      await tab.keyboard.press('ArrowDown');
      await expect.poll(focused).toBe('lead');
      expect(await effort(tab)).toBe('quick');
    } finally {
      await tab.close();
    }
  });
});
