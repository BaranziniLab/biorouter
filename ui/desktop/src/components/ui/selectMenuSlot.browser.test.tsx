// @vitest-environment node
/**
 * A portalled Select menu names itself in the page while it is open (WS-MOTION-2, V1-R7-1).
 *
 * The toast layer yields to an open menu by `data-slot` + `data-state="open"` (main.css, "toast
 * yields to an open menu"), read through `body:has(...)`. A Select that escapes a clipping box
 * (`menuPortalTarget={document.body}`, the Scheduler's cron picker) draws its menu straight under
 * `<body>` at `--z-modal-dropdown`, the same tier as a Radix menu, so it is the menu that slot is
 * for. jsdom never draws a portalled menu (react-select waits for a measured control), so this runs
 * in Chromium. A bare react-select beside it is the instrument: the same open, no slot.
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
import ReactSelect from 'react-select';
import { Select } from './Select';

const options = [
  { value: 'hourly', label: 'Every hour' },
  { value: 'daily', label: 'Every day' },
];

function App() {
  return (
    <div style={{ padding: 24, display: 'grid', gap: 24, width: 320 }}>
      <div id="shared-box" style={{ overflow: 'hidden', height: 48 }}>
        <Select
          classNamePrefix="shared"
          aria-label="Repeat"
          options={options}
          menuPortalTarget={document.body}
          menuPlacement="auto"
        />
      </div>
      <div id="bare-box" style={{ overflow: 'hidden', height: 48 }}>
        <ReactSelect
          classNamePrefix="bare"
          aria-label="Bare"
          options={options}
          menuPortalTarget={document.body}
        />
      </div>
    </div>
  );
}
createRoot(document.getElementById('root')!).render(<App />);
`;

const OPEN_SLOT = "[data-slot='select-content'][data-state='open']";

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

describe('a portalled Select menu names itself while open (V1-R7-1)', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Select menu slot tests.');
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
    const tab = await browser!.newPage({ viewport: { width: 600, height: 400 } });
    await tab.setContent(
      '<!doctype html><html><head></head><body style="margin:0"><div id="root"></div></body></html>'
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('.shared__control');
    await tab.waitForSelector('.bare__control');
    return tab;
  }

  /** What the toast rule's `body:has(...)` sees, and where the slotted node lives. */
  const page = (tab: Page) =>
    tab.evaluate((slot) => {
      const node = document.querySelector(slot);
      return {
        bodyHasOpenMenu: document.body.matches(`body:has(${slot})`),
        slotted: document.querySelectorAll(slot).length,
        isMenu: node?.matches('.shared__menu') ?? false,
        insideClippingBox: Boolean(node?.closest('#shared-box')),
        portalParentIsBody: node?.closest('.shared__menu-portal')?.parentElement === document.body,
      };
    }, OPEN_SLOT);

  it('the bare react-select, opened the same way, names nothing (the instrument)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.click('.bare__control');
      await tab.waitForSelector('.bare__menu');
      expect(await page(tab)).toMatchObject({ bodyHasOpenMenu: false, slotted: 0 });
    } finally {
      await tab.close();
    }
  });

  it('marks the portalled menu under <body> while open, and nothing once it closes', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      expect(await page(tab)).toMatchObject({ bodyHasOpenMenu: false, slotted: 0 });
      await tab.click('.shared__control');
      await tab.waitForSelector('.shared__menu');
      expect(await page(tab)).toEqual({
        bodyHasOpenMenu: true,
        slotted: 1,
        isMenu: true,
        insideClippingBox: false,
        portalParentIsBody: true,
      });
      await tab.keyboard.press('Escape');
      await tab.waitForSelector('.shared__menu', { state: 'detached' });
      expect(await page(tab)).toMatchObject({ bodyHasOpenMenu: false, slotted: 0 });
    } finally {
      await tab.close();
    }
  });
});
