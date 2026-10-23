// @vitest-environment node
/**
 * The effort segments inside a real Radix menu: labels occupy their clickable segments,
 * the shared thumb slides between selections, and menu navigation can enter and leave the group.
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
  const control = React.useRef<HTMLDivElement>(null);
  (window as unknown as { __effort: () => string }).__effort = () => getReasoningEffort(SCOPE);
  return (
    <div style={{ position: 'fixed', left: 40, bottom: 40 }}>
      <DropdownMenu>
        <DropdownMenuTrigger id="chip">Model</DropdownMenuTrigger>
        <DropdownMenuContent
          id="menu"
          side="top"
          align="start"
          onKeyDownCapture={(event) => moveFocusAcrossEffort(event, control.current)}
        >
          <EffortSection scope={SCOPE} effort={effort} controlRef={control} />
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

describe('the model menu’s labelled effort segments in a real layout engine', () => {
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
    await tab.waitForSelector('[data-testid="effort-control"]');
    await tab.waitForTimeout(250);
    return tab;
  }

  const effort = (tab: Page) =>
    tab.evaluate(() => (window as unknown as { __effort: () => string }).__effort());

  it('puts all labels inside full-width segments on the menu’s text edge', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const box = (selector: string) => document.querySelector(selector)!.getBoundingClientRect();
        const section = box('[data-testid="effort-section"]');
        const label = document.querySelector('.br-effort__label')!;
        const control = box('[data-testid="effort-control"]');
        const selected = box('[role="radio"][aria-checked="true"]');
        const thumb = box('[data-slot="segmented-thumb"]');
        return {
          sectionLeft: section.left,
          sectionRight: section.right,
          labelLeft: label.getBoundingClientRect().left,
          labelSize: getComputedStyle(label).fontSize,
          labelWeight: getComputedStyle(label).fontWeight,
          duplicateValue: document.querySelector('[data-testid="effort-value"]') !== null,
          control,
          head: box('.br-effort__head'),
          selected,
          thumb,
          segments: Array.from(document.querySelectorAll('[role="radio"]')).map((item) => {
            const segment = item.getBoundingClientRect();
            const text = item.querySelector('.br-segmented-label')!.getBoundingClientRect();
            return {
              label: item.textContent,
              width: segment.width,
              height: segment.height,
              inside:
                text.left >= segment.left &&
                text.right <= segment.right &&
                text.top >= segment.top &&
                text.bottom <= segment.bottom,
            };
          }),
        };
      });
      near(m.labelLeft - m.sectionLeft, 12, 'Effort label inset');
      expect(m.labelSize).toBe('12px');
      expect(m.labelWeight).toBe('500');
      expect(m.duplicateValue).toBe(false);
      near(m.control.left - m.sectionLeft, 12, 'control left inset');
      near(m.sectionRight - m.control.right, 12, 'control right inset');
      near(m.control.height, 28, 'shared segmented track height');
      expect(m.control.top).toBeGreaterThanOrEqual(m.head.bottom);
      expect(m.segments.map((segment) => segment.label)).toEqual(['Quick', 'Normal', 'Deep']);
      for (const segment of m.segments) {
        expect(segment.inside).toBe(true);
        near(segment.height, 24, 'shared segment height');
        near(segment.width, (m.control.width - 4) / 3, 'equal segment width');
      }
      near(m.thumb.left, m.selected.left, 'thumb starts at Normal');
      near(m.thumb.width, m.selected.width, 'thumb covers the selected segment');
    } finally {
      await tab.close();
    }
  });

  it('applies clicked labels immediately, keeps the menu open, and slides the shared thumb', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.emulateMedia({ reducedMotion: 'no-preference' });
      const thumb = tab.locator('[data-slot="segmented-thumb"]');
      const before = (await thumb.boundingBox())!;
      const transition = await thumb.evaluate((node) => getComputedStyle(node).transitionProperty);
      expect(transition).toContain('translate');
      await tab.getByRole('radio', { name: 'Deep', exact: true }).click();
      expect(await effort(tab)).toBe('deep');
      await tab.waitForTimeout(250);
      const after = (await thumb.boundingBox())!;
      expect(after.x).toBeGreaterThan(before.x);
      const deep = (await tab.getByRole('radio', { name: 'Deep', exact: true }).boundingBox())!;
      near(after.x, deep.x, 'thumb lands under Deep');
      await tab.getByRole('radio', { name: 'Quick', exact: true }).click();
      expect(await effort(tab)).toBe('quick');
      expect(await tab.locator('[data-testid="effort-section"]').count()).toBe(1);
      expect(
        await tab.getByRole('radio', { name: 'Quick', exact: true }).getAttribute('aria-checked')
      ).toBe('true');
    } finally {
      await tab.close();
    }
  });

  it('carries focus between menu rows and the selected segment without changing effort', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const focused = () =>
        tab.evaluate(() => {
          const el = document.activeElement;
          return el?.getAttribute('role') === 'radio' ? el.textContent : (el?.id ?? null);
        });
      await tab.focus('#change');
      await tab.keyboard.press('ArrowUp');
      expect(await focused()).toBe('Normal');
      expect(await effort(tab)).toBe('normal');
      await tab.keyboard.press('ArrowUp');
      expect(await focused()).toBe('Normal');
      expect(await effort(tab)).toBe('normal');
      await tab.keyboard.press('ArrowRight');
      await expect.poll(focused).toBe('Deep');
      expect(await effort(tab)).toBe('deep');
      await tab.keyboard.press('Home');
      await expect.poll(focused).toBe('Quick');
      expect(await effort(tab)).toBe('quick');
      await tab.keyboard.press('End');
      await expect.poll(focused).toBe('Deep');
      expect(await effort(tab)).toBe('deep');
      await tab.keyboard.press('ArrowDown');
      expect(await focused()).toBe('change');
      await tab.keyboard.press('ArrowUp');
      expect(await focused()).toBe('Deep');
      expect(await effort(tab)).toBe('deep');
      await tab.keyboard.press('ArrowDown');
      await tab.keyboard.press('ArrowDown');
      await expect.poll(focused).toBe('lead');
      expect(await effort(tab)).toBe('deep');
    } finally {
      await tab.close();
    }
  });
});
