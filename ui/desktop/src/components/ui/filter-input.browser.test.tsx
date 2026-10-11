// @vitest-environment node
/**
 * The filter field's tooltip is a hover hint for a field the person is not in (vision round 2,
 * V1-R2-7), measured with real pointer and keyboard input in Chromium.
 *
 * History's filter showed "Search history · ⌘F" under the band while the person typed in it, and
 * the tooltip stayed after the pointer left: a click into the field, a move away and back (the
 * pointer crossing the band on the way to the list), and Radix opened the hover tooltip on a
 * focused field; a pointer that then leaves in one jump does not close a hoverable tooltip until
 * it moves again. A Tab into the field opened it too. Both times it repeated what the field
 * already says, and History's placeholder ("Search history… ⌘F") already carries the shortcut.
 *
 * jsdom has no pointer model and no layout, so this runs the real component in Chromium, as the
 * other `*.browser.test.tsx` files do. The first case is the instrument check: hovering a field
 * the person is not in opens the hint, so a case that sees no tooltip is not blind.
 */
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { FilterInput } from './filter-input';
import { AppTooltipProvider } from './Tooltip';

function Field({ id, placeholder, label, findShortcut }) {
  const [value, setValue] = React.useState('');
  return (
    <div id={id} style={{ width: 240 }}>
      <FilterInput
        value={value}
        onValueChange={setValue}
        label={label}
        placeholder={placeholder}
        findShortcut={findShortcut}
      />
    </div>
  );
}

function App() {
  return (
    <AppTooltipProvider>
      <div style={{ padding: 40, display: 'grid', gap: 48, justifyItems: 'start' }}>
        <button id="before" type="button">Before</button>
        <Field id="plain" />
        <Field id="history" label="Search history" placeholder="Search history… ⌘F" />
        <Field id="noshortcut" findShortcut={false} />
      </div>
      <div id="far" style={{ height: 300 }} />
    </AppTooltipProvider>
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

/** Past the app's 500ms label delay. */
const PAST_DELAY = 800;

describe('the filter field’s tooltip is a hover hint for a field the person is not in', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the filter tooltip tests.');
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
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 600, height: 700 } });
    await tab.setContent(
      '<!doctype html><html><head></head><body style="margin:0"><div id="root"></div></body></html>'
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#plain input');
    return tab;
  }

  /** The open tooltip's text (Radix's `role="tooltip"` copy, which holds it once), or null. */
  const tooltip = (tab: Page) =>
    tab.evaluate(() =>
      document.querySelector('[data-slot="tooltip-content"]')
        ? (document.querySelector('[role="tooltip"]')?.textContent ?? '')
        : null
    );

  /** The pointer's way out: one jump far below, as a hand flicking to the list does. */
  const leave = (tab: Page) => tab.mouse.move(300, 650, { steps: 1 });

  it('opens on hover over a field the person is not in (instrument check)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.hover('#plain input');
      await tab.waitForTimeout(PAST_DELAY);
      expect(await tooltip(tab)).toMatch(/^Filter · (⌘F|Ctrl\+F)$/);
    } finally {
      await tab.close();
    }
  });

  it('stays shut while the person is in the field: click, leave, come back, leave', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.click('#plain input');
      await tab.keyboard.type('app:');
      await leave(tab);
      const box = (await tab.locator('#plain input').boundingBox())!;
      await tab.mouse.move(box.x + 24, box.y + box.height / 2, { steps: 4 });
      await tab.waitForTimeout(PAST_DELAY);
      expect(await tooltip(tab), 'over the field it is in').toBeNull();
      await leave(tab);
      await tab.waitForTimeout(PAST_DELAY);
      expect(await tooltip(tab), 'after the pointer left').toBeNull();
      expect(await tab.evaluate(() => document.activeElement?.tagName)).toBe('INPUT');
    } finally {
      await tab.close();
    }
  });

  it('closes a hint the pointer opened once the field takes focus', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.hover('#plain input');
      await tab.waitForTimeout(PAST_DELAY);
      expect(await tooltip(tab)).not.toBeNull();
      await tab.mouse.down();
      await tab.mouse.up();
      await tab.keyboard.type('skill');
      await tab.waitForTimeout(PAST_DELAY);
      expect(await tooltip(tab)).toBeNull();
    } finally {
      await tab.close();
    }
  });

  it('opens on no focus, a Tab included, and names the shortcut to assistive tech instead', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.focus('#before');
      await tab.keyboard.press('Tab');
      expect(await tab.evaluate(() => document.activeElement?.closest('#plain') !== null)).toBe(
        true
      );
      await tab.waitForTimeout(PAST_DELAY);
      expect(await tooltip(tab)).toBeNull();
      const shortcut = await tab.getAttribute('#plain input', 'aria-keyshortcuts');
      expect(shortcut).toMatch(/^(Meta|Control)\+F$/);
    } finally {
      await tab.close();
    }
  });

  it('says the shortcut once: a placeholder that carries it has no tooltip', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      expect(await tab.getAttribute('#history input', 'placeholder')).toBe('Search history… ⌘F');
      await tab.hover('#history input');
      await tab.waitForTimeout(PAST_DELAY);
      expect(await tooltip(tab)).toBeNull();
    } finally {
      await tab.close();
    }
  });

  it('promises no shortcut it does not have: with ⌘F off there is no hint and no key', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.hover('#noshortcut input');
      await tab.waitForTimeout(PAST_DELAY);
      expect(await tooltip(tab)).toBeNull();
      expect(await tab.getAttribute('#noshortcut input', 'aria-keyshortcuts')).toBeNull();
    } finally {
      await tab.close();
    }
  });
});
