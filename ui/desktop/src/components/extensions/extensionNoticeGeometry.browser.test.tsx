// @vitest-environment node
/**
 * The standing load-failure notice, measured in a real layout engine (R-27 polish review).
 *
 * A `Note` centres its action on a 16px first line (`.br-note-action`, calibrated for 12px
 * prose). This notice opens with a 20px title line, so its two text actions and the dismiss mark
 * hung 2px above the title's middle (78 against 80 in the browser), with the glyph already on 80.
 * jsdom has no layout, so the middle of the title and the middle of each action can only be
 * compared here. This renders the real `ExtensionLoadFailureNotice` to markup and lays it out in
 * Chromium with the Note's own action rule cut from `main.css` and `extension-notice.css`.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { recordExtensionLoadResults } from '../../utils/extensionLoadFailures';
import { ExtensionLoadFailureNotice } from './ExtensionLoadFailureNotice';

const here = dirname(fileURLToPath(import.meta.url));
const main = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const noticeCss = readFileSync(resolve(here, 'extension-notice.css'), 'utf8');

/** The Note's own action rule, which the notice's rule has to be measured against. */
function noteActionCss(): string {
  const match = main.match(/\n\.br-note-action \{[^}]*\}/);
  if (!match) throw new Error('.br-note-action rule not found in main.css');
  return match[0];
}

/** `main.css`'s values for the tokens and the utilities the notice's markup uses. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, sans-serif;
  --text-supporting--line-height: 16px;
  --control-sm: 28px;
  --radius-element: 8px;
}
* { box-sizing: border-box; }
body { margin: 0; font: 14px/20px var(--font-body); }
button { font: inherit; margin: 0; padding: 0; border: 0; background: none; }
.flex { display: flex; } .inline-flex { display: inline-flex; } .flex-1 { flex: 1 1 0%; }
.items-start { align-items: flex-start; } .items-center { align-items: center; }
.justify-center { justify-content: center; } .min-w-0 { min-width: 0; } .shrink-0 { flex-shrink: 0; }
.gap-2 { gap: 8px; } .gap-1\\.5 { gap: 6px; } .px-3 { padding-inline: 12px; } .py-2\\.5 { padding-block: 10px; }
.mt-0\\.5 { margin-top: 2px; } .mt-1 { margin-top: 4px; } .mb-4 { margin-bottom: 16px; }
.h-4 { height: 16px; } .w-4 { width: 16px; } .h-control-sm { height: 28px; } .w-control-sm { width: 28px; }
.p-0 { padding: 0; } .flex-col { flex-direction: column; }
.rounded-element { border-radius: 8px; }
.text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
.text-supporting { font-size: 12px; line-height: 16px; }
svg { flex: none; }`;

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

// 90s a test: a page launch alone takes seconds while other browser suites load the machine.
describe('The extension load-failure notice in a real layout engine', { timeout: 90_000 }, () => {
  let browser: Browser | null = null;
  let html = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the notice geometry test. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
    recordExtensionLoadResults([
      { name: 'cdwagent', success: false, error: 'failed to start the MCP server' },
    ]);
    const markup = renderToStaticMarkup(<ExtensionLoadFailureNotice onAskBiorouter={() => {}} />);
    html = `<!doctype html><html><head><style>${TOKENS}\n${noteActionCss()}\n${noticeCss}</style></head><body><div style="width:712px">${markup}</div></body></html>`;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 400 } });
    await tab.setContent(html);
    return tab;
  }

  it('centres Ask Biorouter, Copy error and the dismiss mark on the title line, as the glyph is', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const mid = (el: Element) => {
          const r = el.getBoundingClientRect();
          return r.top + r.height / 2;
        };
        const note = document.querySelector('[role="alert"]')!;
        return {
          title: mid(note.querySelector('.text-label')!),
          glyph: mid(note.querySelector('svg')!),
          actions: [...note.querySelectorAll('.br-ext-notice-actions button')].map(mid),
          height: note.getBoundingClientRect().height,
        };
      });
      expect(m.actions).toHaveLength(3);
      expect(Math.abs(m.glyph - m.title)).toBeLessThan(0.5);
      for (const action of m.actions) expect(Math.abs(action - m.title)).toBeLessThan(0.5);
      // The nudge is padding inside the slot's negative margin: the notice is as tall as its
      // text (10 + 20 + 4 + 16 + 10), not taller for carrying actions.
      expect(m.height).toBe(60);
    } finally {
      await tab.close();
    }
  });
});
