// @vitest-environment node
/**
 * The page band measured in a real layout engine (spec 3.10 and 5.7).
 *
 * jsdom lays nothing out and evaluates no sibling combinator against a sidebar's state, so the
 * component test (`PageHeader.test.tsx`) can only pin class names. The acceptance numbers are
 * geometry: the band is 44px (`--chrome-height`) with its hairline at y=44, where it meets the
 * sidebar's titlebar band and the chat header; the title starts 16px in with the sidebar open,
 * and at the titlebar reserve with it collapsed, so the floating titlebar controls never sit
 * on the title; and the bar (not the header) is the window-drag rect, every control in it
 * `no-drag`, the reserve outside the drag rect (issue #74).
 *
 * The real `PageHeader` is bundled with esbuild and rendered by React in Chromium, styled by
 * the `.biorouter-page-header` family of `styles/main.css` (this workstream's `@ws WS-VIEWS-A`
 * blocks plus the hairline rule above them) and WS-PRIMITIVES' blocks for the InfoTip, with the
 * token values read from that file.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

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

/** The band's hairline rule (D-05), which sits just above this workstream's markers. */
function hairlineRule(): string {
  const match = MAIN_CSS.match(/\n\.biorouter-page-header \{[^}]*\}/);
  if (!match) throw new Error('main.css has no `.biorouter-page-header` hairline rule');
  return match[0];
}

/**
 * The Tabs primitive's own rules (`.br-tabs-list`, `.br-tabs-indicator`), cut from main.css so the
 * real indicator hangs from the real list. Tailwind's `@layer utilities` is what the primitive's
 * trigger height (`h-control-lg`) arrives in, so the few utilities the markup uses are declared
 * inside that layer here too: an unlayered band rule beats them whatever its specificity, and a
 * test that declared them unlayered would pass for a reason the app does not have.
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
  --background-focus: #ddd; --border-focus: #555; }
[data-slot='shell'] { display: flex; width: 1200px; height: 400px; }
[data-slot='sidebar'][data-state='expanded'] { width: 288px; flex: none; }
[data-slot='sidebar'][data-state='collapsed'] { width: 0; flex: none; }
[data-slot='sidebar-inset'] { flex: 1; min-width: 0; display: flex; flex-direction: column; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { PageHeader, PageHeaderAction } from './PageHeader';
import { Upload } from '../icons/app-icons';
import { Tabs, TabsList, TabsTrigger } from '../ui/tabs';

function Shell({ state, onBack }: { state: 'expanded' | 'collapsed'; onBack?: boolean }) {
  return (
    <div data-slot="shell" id={state + (onBack ? '-back' : '')}>
      <div data-slot="sidebar" data-state={state} />
      <main data-slot="sidebar-inset">
        <PageHeader
          title="Workflows"
          info="Reusable chat setups."
          onBack={onBack ? () => {} : undefined}
          actions={
            <>
              <input aria-label="Filter workflows" style={{ width: 160, height: 28 }} />
              <PageHeaderAction icon={Upload} label="Import workflow" onClick={() => {}} />
              <button type="button" style={{ height: 32 }}>New workflow</button>
            </>
          }
        />
      </main>
    </div>
  );
}

/** A band with tabs, as Knowledge and Settings pass them: a \`TabsList\` with no rule of its own. */
function TabsShell() {
  return (
    <div data-slot="shell" id="tabs">
      <div data-slot="sidebar" data-state="expanded" />
      <main data-slot="sidebar-inset">
        <Tabs defaultValue="graph">
          <PageHeader
            title="Knowledge"
            tabs={
              <TabsList className="border-b-0">
                <TabsTrigger value="digest">Sources</TabsTrigger>
                <TabsTrigger value="graph">Graph</TabsTrigger>
              </TabsList>
            }
          />
        </Tabs>
      </main>
    </div>
  );
}

function App() {
  return (
    <div>
      <Shell state="expanded" />
      <Shell state="collapsed" />
      <Shell state="expanded" onBack />
      <TabsShell />
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

describe('the page band in a real layout engine (spec 3.10, 5.7)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the page band geometry tests.'
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
    css =
      PREFLIGHT +
      readTokens() +
      wsBlocks('WS-PRIMITIVES') +
      hairlineRule() +
      wsBlocks('WS-VIEWS-A') +
      tabsCss();
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(reserve?: number): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1200, height: 1300 } });
    const root =
      reserve === undefined ? '' : `:root { --biorouter-titlebar-control-reserve: ${reserve}px; }`;
    await tab.setContent(
      `<!doctype html><html><head><style>${css}${root}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#expanded-back header');
    return tab;
  }

  /** Boxes relative to each shell's own top-left, so the three shells read alike. */
  async function measure(tab: Page, shellId: string) {
    return tab.evaluate((id) => {
      const shell = document.getElementById(id)!;
      const origin = shell.getBoundingClientRect();
      const rel = (el: Element) => {
        const r = el.getBoundingClientRect();
        return { x: r.left - origin.left, y: r.top - origin.top, w: r.width, h: r.height };
      };
      const header = shell.querySelector('header.biorouter-page-header')!;
      const bar = header.querySelector('.biorouter-page-header-bar')!;
      const title = header.querySelector('h1')!;
      const inset = shell.querySelector('[data-slot="sidebar-inset"]')!;
      const region = (el: Element) =>
        getComputedStyle(el).getPropertyValue('-webkit-app-region').trim();
      const buttons = Array.from(bar.querySelectorAll('button, input'));
      return {
        header: rel(header),
        bar: rel(bar),
        title: rel(title),
        inset: rel(inset),
        hairline: getComputedStyle(header).borderBottomWidth,
        headerRegion: region(header),
        barRegion: region(bar),
        controlRegions: buttons.map(region),
      };
    }, shellId);
  }

  it('is 44px tall with its hairline at y=44', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      for (const id of ['expanded', 'collapsed', 'expanded-back']) {
        const m = await measure(tab, id);
        near(m.header.y, 0, `${id} band top`);
        near(m.header.h, 44, `${id} band height`);
        near(m.header.y + m.header.h, 44, `${id} hairline bottom`);
        expect(m.hairline, `${id} hairline width`).toBe('1px');
        // The hairline and the ground run edge to edge of the pane.
        near(m.header.x, m.inset.x, `${id} band left edge`);
        near(m.header.w, m.inset.w, `${id} band width`);
      }
    } finally {
      await tab.close();
    }
  });

  it('starts the title 16px in with the sidebar open', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab, 'expanded');
      near(m.title.x - m.inset.x, 16, 'title inset');
      // Vertically centred in the band.
      near(m.title.y + m.title.h / 2, 22, 'title centre');
    } finally {
      await tab.close();
    }
  });

  it('starts the title at the titlebar reserve with the sidebar collapsed', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const reserve of [undefined, 132]) {
      const tab = await open(reserve);
      try {
        const m = await measure(tab, 'collapsed');
        const expected = reserve ?? 172;
        near(m.title.x - m.inset.x, expected, `title x with a ${expected}px reserve`);
        // The reserve is a MARGIN: the drag rect starts after it, never under the controls.
        near(m.bar.x - m.inset.x, expected, `drag rect left with a ${expected}px reserve`);
      } finally {
        await tab.close();
      }
    }
  });

  it('puts the back button first, then the title, in a drill-in', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab, 'expanded-back');
      const plain = await measure(tab, 'expanded');
      const back = await tab.evaluate(() => {
        const button = document.querySelector('#expanded-back button[aria-label="Back"]')!;
        const r = button.getBoundingClientRect();
        return { left: r.left, right: r.right };
      });
      // The button's own 32px box comes from Tailwind, which this page does not run; what the
      // band owns is the order and the 8px gap after it.
      near(back.left, plain.title.x, 'back button starts where the title would');
      near(m.title.x - back.right, 8, 'gap between back and title');
    } finally {
      await tab.close();
    }
  });

  it('drags the window from the bar, and never from its controls', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab, 'collapsed');
      // Chromium parses the property outside Electron too; if it ever stops, the numbers above
      // still hold and this is the one assertion that cannot run here.
      if (!m.barRegion) return ctx.skip();
      expect(m.barRegion).toBe('drag');
      expect(m.headerRegion).not.toBe('drag');
      expect(m.controlRegions.length).toBeGreaterThan(0);
      for (const region of m.controlRegions) expect(region).toBe('no-drag');
    } finally {
      await tab.close();
    }
  });

  // V3-R5-2: a tab's label sat 3.5px under the title's line, because the tab wrapper
  // bottom-aligned the primitive's 36px trigger in the 43px bar the title is centred in. The
  // triggers now fill the bar. Measured with the real Tabs primitive after it has placed its
  // indicator, so the underline is the one the app draws.
  async function measureTabs(tab: Page) {
    await tab.waitForSelector('#tabs .br-tabs-indicator[data-placed="true"]');
    return tab.evaluate(() => {
      const shell = document.getElementById('tabs')!;
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
        return { top: r.top, bottom: r.bottom, left: r.left, width: r.width };
      };
      const active = shell.querySelector('[role="tab"][data-state="active"]')!;
      return {
        header: rect(header),
        bar: rect(bar),
        title: ink(header.querySelector('h1')!),
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

  it('sets a tab label on the title line, with the underline on the hairline', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measureTabs(tab);
      expect(m.tabs).toHaveLength(2);
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

  // The instrument: with the band's two tab rules put back to what they were (the wrapper
  // bottom-aligning the trigger at its own 36px), the same measurement reads the 3.5px the
  // vision pass found. A test that cannot fail says nothing.
  it('measures the old offset when the tab rules are removed', async (ctx) => {
    if (!browser) return ctx.skip();
    const fill = /\.biorouter-page-header-tabs > \[role='tablist'\] > \[role='tab'\] \{[^}]*\}/;
    const stretch = /(\.biorouter-page-header-tabs \{[^}]*?align-items:\s*)stretch/;
    expect(css, 'the trigger rule is in the stylesheet').toMatch(fill);
    expect(css, 'the wrapper stretches its list').toMatch(stretch);
    const old = css.replace(fill, '').replace(stretch, '$1flex-end');
    const tab = await browser.newPage({ viewport: { width: 1200, height: 1300 } });
    try {
      await tab.setContent(
        `<!doctype html><html><head><style>${old}</style></head><body><div id="root"></div></body></html>`
      );
      await tab.addScriptTag({ content: script });
      const m = await measureTabs(tab);
      for (const t of m.tabs) {
        near(t.ink.mid - m.title.mid, 3.5, 'the old label offset below the title');
        near(t.bottom - t.top, 36, 'the old trigger height');
      }
    } finally {
      await tab.close();
    }
  });
});
