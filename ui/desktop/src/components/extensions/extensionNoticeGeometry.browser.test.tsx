// @vitest-environment node
/**
 * The standing load-failure notice, measured in a real layout engine (R-27 polish review).
 *
 * A `Note` centres its action on a 16px first line (`.br-note-action`, calibrated for 12px
 * prose). This notice opens with a 20px title line, so its two text actions and the dismiss mark
 * hung 2px above the title's middle (78 against 80 in the browser), with the glyph already on 80.
 * They now sit in the notice's own grid, on the title's row (so they can step aside from a
 * toast, final smoke), centred on it with the row kept at 20px. jsdom has no layout, so the
 * middle of the title and the middle of each action can only be compared here. This renders the
 * real `ExtensionLoadFailureNotice` to markup and lays it out in Chromium with the Note's own
 * action rule cut from `main.css` and `extension-notice.css`.
 *
 * Second case (vision round 6, V3-R4-5): Copy error turned into the narrower Copied for two
 * seconds and Ask Biorouter, beside it, slid 22px right and back. That one needs the click, so
 * the real component is bundled with esbuild, mounted in Chromium and pressed. The instrument
 * bundles it again with the label kept only to hold the width removed, and must read the shift.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { build, type Plugin } from 'esbuild';
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

/**
 * The real notice, mounted and live. `dropWidthLabel` takes out the second label that holds the
 * button's width (the instrument), and throws when the line it removes is no longer there.
 */
const LIVE_ENTRY = `
import { createRoot } from 'react-dom/client';
import { recordExtensionLoadResults } from '../../utils/extensionLoadFailures';
import { ExtensionLoadFailureNotice } from './ExtensionLoadFailureNotice';

recordExtensionLoadResults([
  { name: 'cdwagent', success: false, error: 'failed to start the MCP server' },
]);
createRoot(document.getElementById('root')!).render(
  <ExtensionLoadFailureNotice onAskBiorouter={() => {}} />
);
`;

async function bundleLive(dropWidthLabel: boolean): Promise<string> {
  const mutate: Plugin = {
    name: 'drop-width-label',
    setup(b) {
      if (!dropWidthLabel) return;
      b.onLoad({ filter: /\/ExtensionLoadFailureNotice\.tsx$/ }, (args) => {
        const source = readFileSync(args.path, 'utf8');
        const from = '<span aria-hidden="true">{other}</span>';
        if (!source.includes(from)) throw new Error('mutation target missing in the notice');
        return {
          contents: source.replace(from, 'null'),
          loader: 'tsx',
          resolveDir: dirname(args.path),
        };
      });
    },
  };
  const out = await build({
    stdin: { contents: LIVE_ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
    bundle: true,
    write: false,
    format: 'iife',
    jsx: 'automatic',
    platform: 'browser',
    define: { 'process.env.NODE_ENV': '"production"' },
    loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
    logLevel: 'silent',
    plugins: [mutate],
  });
  return out.outputFiles[0].text;
}

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
  let live = '';
  let liveWithoutWidthLabel = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the notice geometry test. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
    [live, liveWithoutWidthLabel] = await Promise.all([bundleLive(false), bundleLive(true)]);
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

  /** Mount the live notice with the clipboard stubbed, and press Copy error. */
  async function pressCopy(script: string) {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 400 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${TOKENS}\n${noteActionCss()}\n${noticeCss}</style></head>` +
        '<body><div id="root" style="width:712px"></div></body></html>'
    );
    await tab.evaluate(() => {
      Object.defineProperty(navigator, 'clipboard', {
        configurable: true,
        value: { writeText: async () => undefined },
      });
    });
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('.br-ext-notice-actions button');
    const read = () =>
      tab.evaluate(() => {
        const buttons = [...document.querySelectorAll('.br-ext-notice-actions button')];
        const [ask, copy] = buttons.map((b) => b.getBoundingClientRect());
        const shown = document.querySelector('.br-ext-notice-swap > span:first-child')!;
        const text = shown.getBoundingClientRect();
        return {
          label: shown.textContent,
          askLeft: ask.left,
          copyLeft: copy.left,
          copyWidth: copy.width,
          labelCentre: text.left + text.width / 2,
          buttonCentre: copy.left + copy.width / 2,
        };
      });
    const before = await read();
    await tab.getByRole('button', { name: 'Copy error' }).click();
    await tab.waitForFunction(
      () =>
        document.querySelector('.br-ext-notice-swap > span:first-child')?.textContent === 'Copied'
    );
    const during = await read();
    // Two seconds later the label is back; the button must not have moved on the way back either.
    await tab.waitForFunction(
      () =>
        document.querySelector('.br-ext-notice-swap > span:first-child')?.textContent ===
        'Copy error',
      undefined,
      { timeout: 6000 }
    );
    const after = await read();
    return { tab, before, during, after };
  }

  it('keeps Ask Biorouter and the Copy error button where they are while it says Copied (V3-R4-5)', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, before, during, after } = await pressCopy(live);
    try {
      expect(before.label).toBe('Copy error');
      expect(during.label).toBe('Copied');
      expect(after.label).toBe('Copy error');
      // The button keeps its width, so the neighbour on its left does not move (it was 22px).
      expect(Math.abs(during.copyWidth - before.copyWidth)).toBeLessThan(0.5);
      expect(Math.abs(during.askLeft - before.askLeft)).toBeLessThan(0.5);
      expect(Math.abs(during.copyLeft - before.copyLeft)).toBeLessThan(0.5);
      expect(Math.abs(after.askLeft - before.askLeft)).toBeLessThan(0.5);
      expect(Math.abs(after.copyWidth - before.copyWidth)).toBeLessThan(0.5);
      // The shorter word sits in the middle of the button it keeps, not against one edge.
      expect(Math.abs(during.labelCentre - during.buttonCentre)).toBeLessThan(0.5);
      expect(Math.abs(before.labelCentre - before.buttonCentre)).toBeLessThan(0.5);
    } finally {
      await tab.close();
    }
  });

  // The instrument: the same press with the width-holding label taken out must read the old shift,
  // or the case above could pass on a layout that does not look at the button at all.
  it('reads the 22px shift when the label that holds the width is taken out', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, before, during } = await pressCopy(liveWithoutWidthLabel);
    try {
      expect(before.copyWidth - during.copyWidth).toBeGreaterThan(15);
      expect(during.askLeft - before.askLeft).toBeGreaterThan(15);
    } finally {
      await tab.close();
    }
  });
});
