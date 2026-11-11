// @vitest-environment node
/**
 * The ScrollArea's growth follow in a real layout engine (G-7, live QA 2026-11-10).
 *
 * The follow used to run only when the scroll area's `children` changed. A disclosure that opens
 * inside a row keeps its open state in that row, so opening it re-renders the row alone: the
 * transcript's children are the same elements, nothing asked the follow, and the opened panel
 * grew under the composer of a view that was following the end. jsdom lays nothing out and has no
 * ResizeObserver, so this is measured in Chromium: the real component, bundled with esbuild,
 * rendered by React, with a row whose own state opens a 240px panel.
 */
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type BrowserContext, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));

const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; background: #fff; }
.relative { position: relative; }
.overflow-hidden { overflow: hidden; }
.absolute { position: absolute; }
.h-full { height: 100%; }
.w-full { width: 100%; }
.row { height: 40px; }
.panel { height: 240px; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ScrollArea } from './scroll-area';

/** A row that holds its own open state, like the decide card and a decision turn marker. */
function DisclosureRow() {
  const [open, setOpen] = React.useState(false);
  return (
    <div>
      <button id="toggle" type="button" className="row" onClick={() => setOpen((o) => !o)}>
        Details
      </button>
      {open && <div id="panel" className="panel">Opened</div>}
    </div>
  );
}

// Created once, so the scroll area's children never change after the first commit.
const rows = Array.from({ length: 20 }, (_, i) => <div key={i} className="row">Row {i}</div>);
const transcript = (
  <>
    {rows}
    <DisclosureRow />
  </>
);

function App() {
  return (
    <ScrollArea id="area" autoScroll style={{ height: 300, width: 400 }}>
      {transcript}
    </ScrollArea>
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

describe('the ScrollArea follow in a real layout engine (G-7)', () => {
  let browser: Browser | null = null;
  let script = '';
  const contexts: BrowserContext[] = [];

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the follow tests.');
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
    for (const context of contexts) await context.close().catch(() => {});
    await browser?.close();
  }, 120_000);

  /** A couple of frames, then past the 100ms "actively scrolling" window a follow leaves. */
  const settle = (tab: Page) =>
    tab.evaluate(
      () =>
        new Promise((done) =>
          requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(done, 150)))
        )
    );

  async function open(): Promise<Page> {
    const context = await browser!.newContext({ viewport: { width: 600, height: 600 } });
    contexts.push(context);
    const tab = await context.newPage();
    await tab.setContent(
      `<!doctype html><html><head><style>${PREFLIGHT}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#toggle');
    await settle(tab);
    return tab;
  }

  /** The viewport's distance from its end, and its scroll position. */
  const geometry = (tab: Page) =>
    tab.evaluate(() => {
      const viewport = document.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement;
      return {
        top: viewport.scrollTop,
        fromEnd: viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight,
        height: viewport.scrollHeight,
      };
    });

  it('keeps a view that follows the end at the end when a row opens its own disclosure', async () => {
    if (!browser) return;
    const tab = await open();
    const before = await geometry(tab);
    expect(before.fromEnd, 'the view starts at its end').toBeLessThanOrEqual(1);

    await tab.click('#toggle');
    await tab.waitForSelector('#panel');
    await settle(tab);

    const after = await geometry(tab);
    expect(after.height - before.height, 'the panel grew the content').toBeGreaterThanOrEqual(239);
    expect(
      after.fromEnd,
      'the opened panel is in view, not under the composer'
    ).toBeLessThanOrEqual(1);
  }, 60_000);

  it('leaves a reader who scrolled up where they are when a row grows', async () => {
    if (!browser) return;
    const tab = await open();
    await tab.evaluate(() => {
      const viewport = document.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement;
      viewport.scrollTop = 0;
    });
    await settle(tab);

    await tab.evaluate(() => (document.getElementById('toggle') as HTMLButtonElement).click());
    await tab.waitForSelector('#panel');
    await settle(tab);

    expect((await geometry(tab)).top).toBe(0);
  }, 60_000);
});
