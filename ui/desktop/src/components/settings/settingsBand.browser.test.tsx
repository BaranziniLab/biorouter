// @vitest-environment node
/**
 * The Settings band's tabs measured in a real layout engine (vision V3-R5-2, spec 3.13).
 *
 * jsdom lays nothing out and applies no stylesheet, so `settingsBand.test.ts` can pin only what is
 * written in the source. What the owner sees is geometry: "Models Chat App" sits on the title's
 * line (the band centres a 14px title in its 43px bar, and a tab label that sat 3.5px under it
 * made the band read as two lines that had slipped), the underline lands on the band's hairline,
 * and the title and the first tab are 24px apart.
 *
 * Who draws what: the SHARED band rule in `main.css` (`.biorouter-page-header-tabs`, WS-VIEWS-A)
 * stretches the wrapper and every direct trigger of a band tab list, so any `PageHeader tabs`
 * list gets the alignment. Settings keeps only what is its own, the 16px margin in `settings.css`.
 * Its two older per-view declarations (`align-self: stretch` on the list, `height: 100%` on the
 * trigger) said the same thing twice, the second one at a lower specificity than the shared rule
 * so it never decided a pixel, and they are gone. This file is what holds the Settings band to
 * the numbers without them.
 *
 * The real `PageHeader` and the real `Tabs` primitive are bundled with esbuild and rendered by
 * React in Chromium, styled by the authored blocks of `styles/main.css` and then `settings.css`,
 * with the props `SettingsView` passes (its class on the list, `border-b-0`).
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const SETTINGS_CSS = readFileSync(resolve(here, 'settings.css'), 'utf8');

const TOKEN_NAMES = ['--chrome-height', '--control-md', '--radius-full', '--radius-inner'];

/** The first declaration of each token in main.css, so a token change reaches this test. */
function readTokens(): string {
  const lines = TOKEN_NAMES.map((name) => {
    const match = MAIN_CSS.match(new RegExp(`\\n\\s*${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
    if (!match) throw new Error(`main.css does not declare ${name}`);
    return `  ${name}: ${match[1].trim()};`;
  });
  return `:root {\n${lines.join('\n')}\n}`;
}

/** Every `@ws <ws>` block of main.css, in file order. */
function wsBlocks(ws: string): string {
  const out: string[] = [];
  const re = new RegExp(
    `/\\* @ws ${ws} begin: ([^*]+?) \\*/([\\s\\S]*?)/\\* @ws ${ws} end: \\1 \\*/`,
    'g'
  );
  for (const match of MAIN_CSS.matchAll(re)) out.push(match[2]);
  if (out.length === 0) throw new Error(`main.css has no @ws ${ws} block`);
  return out.join('\n');
}

/** The band's hairline rule (D-05), which sits just above the WS-VIEWS-A markers. */
function hairlineRule(): string {
  const match = MAIN_CSS.match(/\n\.biorouter-page-header \{[^}]*\}/);
  if (!match) throw new Error('main.css has no `.biorouter-page-header` hairline rule');
  return match[0];
}

/**
 * The Tabs primitive's own rules (`.br-tabs-list`, `.br-tabs-indicator`), cut from main.css so the
 * real indicator hangs from the real list. The trigger's `h-control-lg` arrives in Tailwind's
 * `utilities` layer in the app, so the few utilities the markup uses are declared inside that
 * layer here too: an unlayered band rule beats them whatever its specificity, and a test that
 * declared them unlayered would pass for a reason the app does not have.
 */
function tabsCss(): string {
  const begin = MAIN_CSS.indexOf('@layer components {\n  .br-tabs-list {');
  const endMarker = ".br-tabs-list:has(> [role='tab']:focus-visible) > .br-tabs-indicator {";
  const end = MAIN_CSS.indexOf('}', MAIN_CSS.indexOf(endMarker));
  if (begin < 0 || end < begin) throw new Error('main.css has no tab list rules');
  return `
:root { --accent-bar: #c4452d; --dur-fast-max: 175ms; --dur-fast: 95ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1); --control-lg: 36px; }
@layer utilities {
  .flex { display: flex; } .flex-col { flex-direction: column; } .inline-flex { display: inline-flex; }
  .flex-1 { flex: 1 1 0%; } .min-h-0 { min-height: 0; }
  .items-center { align-items: center; } .gap-5 { gap: 20px; } .relative { position: relative; }
  .h-control-lg { height: var(--control-lg); } .whitespace-nowrap { white-space: nowrap; }
  .border-b-0 { border-bottom-width: 0; }
  .text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
}
${MAIN_CSS.slice(begin, end + 1)}`;
}

/** Tailwind's preflight, the parts the layout depends on, plus colours that only paint. */
const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
h1 { margin: 0; font-size: 14px; line-height: 20px; font-weight: 500; }
button { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
svg { display: block; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
.min-w-0 { min-width: 0; }
.truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
:root { --sidebar: #f7f7f5; --border-subtle: #d9d9d6; --text-subtle: #777; --text-default: #111;
  --text-muted: #666; --background-focus: #ddd; --border-focus: #555; }
[data-slot='shell'] { display: flex; width: 1200px; height: 400px; }
[data-slot='sidebar'][data-state='expanded'] { width: 288px; flex: none; }
[data-slot='sidebar-inset'] { flex: 1; min-width: 0; display: flex; flex-direction: column; }
`;

/** `SettingsView`'s band, prop for prop: the class on the list, `border-b-0`, three triggers. */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { PageHeader } from '../Layout/PageHeader';
import { Tabs, TabsList, TabsTrigger } from '../ui/tabs';

function App() {
  return (
    <div data-slot="shell" id="settings">
      <div data-slot="sidebar" data-state="expanded" />
      <main data-slot="sidebar-inset">
        <Tabs defaultValue="chat" className="flex min-h-0 flex-1 flex-col">
          <PageHeader
            title="Settings"
            tabs={
              <TabsList aria-label="Settings sections" className="biorouter-settings-tabs border-b-0">
                <TabsTrigger value="models">Models</TabsTrigger>
                <TabsTrigger value="chat">Chat</TabsTrigger>
                <TabsTrigger value="app">App</TabsTrigger>
              </TabsList>
            }
          />
        </Tabs>
      </main>
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

describe('the Settings band tabs in a real layout engine (V3-R5-2, spec 3.13)', () => {
  let browser: Browser | null = null;
  let script = '';
  let base = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the Settings band geometry tests.'
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
    // The order the app loads them: main.css's blocks, then the view's own stylesheet.
    base =
      PREFLIGHT +
      readTokens() +
      wsBlocks('WS-PRIMITIVES') +
      hairlineRule() +
      wsBlocks('WS-VIEWS-A') +
      tabsCss();
    css = base + SETTINGS_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(sheet: string): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1200, height: 600 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${sheet}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#settings .br-tabs-indicator[data-placed="true"]');
    return tab;
  }

  async function measure(tab: Page) {
    return tab.evaluate(() => {
      const shell = document.getElementById('settings')!;
      const header = shell.querySelector('header.biorouter-page-header')!;
      const bar = header.querySelector('.biorouter-page-header-bar')!;
      // The ink of a text node, not its box: the title and the labels are one face and size, so
      // the glyph boxes compare like for like.
      const ink = (el: Element) => {
        const range = document.createRange();
        range.selectNodeContents(el);
        const r = range.getBoundingClientRect();
        return { top: r.top, bottom: r.bottom, mid: (r.top + r.bottom) / 2 };
      };
      const rect = (el: Element) => {
        const r = el.getBoundingClientRect();
        return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, width: r.width };
      };
      const active = shell.querySelector('[role="tab"][data-state="active"]')!;
      return {
        header: rect(header),
        bar: rect(bar),
        title: ink(header.querySelector('h1')!),
        titleBox: rect(header.querySelector('.biorouter-page-header-title')!),
        wrapper: rect(header.querySelector('.biorouter-page-header-tabs')!),
        list: rect(shell.querySelector('[role="tablist"]')!),
        indicator: rect(shell.querySelector('.br-tabs-indicator')!),
        active: rect(active),
        tabs: Array.from(shell.querySelectorAll('[role="tab"]')).map((el) => ({
          ...rect(el),
          ink: ink(el),
        })),
      };
    });
  }

  it('sets every tab label on the title line, with the underline on the hairline', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(css);
    try {
      const m = await measure(tab);
      expect(m.tabs).toHaveLength(3);
      for (const t of m.tabs) {
        // One centre line for the title and every label (a pixel is the most a rounding gives).
        expect(Math.abs(t.ink.mid - m.title.mid), 'label vs title midline').toBeLessThan(1);
        // The trigger fills the bar, top to bottom, so it is a full-height target as well.
        near(t.top, m.bar.top, 'trigger top');
        near(t.bottom, m.bar.bottom, 'trigger bottom');
      }
      near(m.list.top, m.bar.top, 'list top');
      near(m.list.bottom, m.bar.bottom, 'list bottom');
      // The bar ends where the band's 1px hairline begins, and the indicator covers that row.
      near(m.bar.bottom, m.header.bottom - 1, 'bar bottom');
      near(m.indicator.bottom, m.header.bottom, 'underline bottom is the hairline');
      near(m.indicator.bottom - m.indicator.top, 2, 'underline thickness');
      // Under the active trigger, not somewhere else.
      near(m.indicator.left, m.active.left, 'underline left');
      near(m.indicator.width, m.active.width, 'underline width');
    } finally {
      await tab.close();
    }
  });

  it('keeps the title and the first tab 24px apart: the bar’s 8px gap and Settings’ own 16px', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(css);
    try {
      const m = await measure(tab);
      near(m.list.left - m.wrapper.left, 16, 'the list’s own margin');
      near(m.list.left - m.titleBox.right, 24, 'title to the first tab');
    } finally {
      await tab.close();
    }
  });

  // The instruments. A guard that cannot fail says nothing, so each half of the claim above is
  // put back to the state it guards against and the same measurement must read the defect.
  it('reads the old 3.5px offset and 36px trigger when the shared band rules are removed', async (ctx) => {
    if (!browser) return ctx.skip();
    const fill = /\.biorouter-page-header-tabs > \[role='tablist'\] > \[role='tab'\] \{[^}]*\}/;
    const stretch = /(\.biorouter-page-header-tabs \{[^}]*?align-items:\s*)stretch/;
    expect(css, 'the shared trigger rule is in the stylesheet').toMatch(fill);
    expect(css, 'the shared wrapper stretches its list').toMatch(stretch);
    const old = css.replace(fill, '').replace(stretch, '$1flex-end');
    const tab = await open(old);
    try {
      const m = await measure(tab);
      for (const t of m.tabs) {
        near(t.ink.mid - m.title.mid, 3.5, 'the old label offset below the title');
        near(t.bottom - t.top, 36, 'the old trigger height');
      }
    } finally {
      await tab.close();
    }
  });

  it('reads 8px, not 24px, between the title and the first tab without Settings’ margin', async (ctx) => {
    if (!browser) return ctx.skip();
    const margin = /margin-inline-start:\s*16px;/;
    expect(SETTINGS_CSS, 'settings.css gives the list its margin').toMatch(margin);
    const tab = await open(base + SETTINGS_CSS.replace(margin, ''));
    try {
      const m = await measure(tab);
      near(m.list.left - m.titleBox.right, 8, 'title to the first tab with no margin');
    } finally {
      await tab.close();
    }
  });
});
