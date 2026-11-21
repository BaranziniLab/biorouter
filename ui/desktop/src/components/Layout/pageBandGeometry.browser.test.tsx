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
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const MAIN_CSS = readFileSync(resolve(STYLES, 'main.css'), 'utf8');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

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

/**
 * EVERY PAGE BAND'S CONTROLS ARE ONE SIZE (owner, 2026-11-19).
 *
 * The coral primaries ("New schedule", "Add extension") and the icon buttons were the default 32px
 * rung beside the 28px filter, so a band held two heights and the primary nearly filled it. The band
 * now points the `md` rung at `sm` for its own descendants (`.biorouter-page-header-bar` in
 * `main.css`), so every control a view passes at its default size draws 28px.
 *
 * Each band below is one page's real action cluster, built from the primitives that page passes
 * (`PageHeaderAction`, `PageHeaderMenu`, `FilterInput`, a `Button` primary, a `Button` inside a
 * `DropdownMenuTrigger`, Extensions' own `AddExtensionMenu`, a ghost text button, a secondary round
 * button, the drill-in's Back) with the page's own copy. Unlike the suite above, the stylesheet is
 * the app's: `styles/main.css` compiled by Tailwind with the classes the bundle uses, so the
 * buttons' `h-control-md`, `px-3`, `gap-2` and `rounded-element` are the real utilities and the
 * cascade between them and the band's unlayered rule is the one the app has. Expected values are
 * read from probes that paint the tokens, so a token change moves the expectation with it.
 */
const GALLERY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { PageHeader, PageHeaderAction, PageHeaderMenu } from './PageHeader';
import { Button } from '../ui/button';
import { FilterInput } from '../ui/filter-input';
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '../ui/dropdown-menu';
import { RowActionMenuItems } from '../ui/row-actions';
import { ArrowRight, ClipboardList, Pause, Plus, RefreshCw, Share2, Upload } from '../icons/app-icons';
import { AddExtensionMenu } from '../extensions/AddExtensionMenu';
import { DICTIONARY_LABELS } from '../dictionary/copy';
import { SKILLS_COPY } from '../skills/copy';
import { scheduleCopy } from '../schedule/copy';
import { workflowsCopy } from '../workflows/copy';

const noop = () => {};
const menu = [{ label: 'Delete…', onSelect: noop, destructive: true }];

function Filter({ label, className }: { label: string; className?: string }) {
  const [value, setValue] = React.useState('');
  return <FilterInput value={value} onValueChange={setValue} label={label} className={className} />;
}

/** The Skills and Dictionary primary: a coral Button as a menu trigger. */
function AddMenu({ label }: { label: string }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button>
          <Plus aria-hidden />
          {label}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <RowActionMenuItems items={menu} />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

const PAGES: Array<{ id: string; title: string; back?: boolean; actions: React.ReactNode }> = [
  {
    id: 'scheduler',
    title: 'Scheduler',
    actions: (
      <>
        <PageHeaderAction icon={RefreshCw} label="Refresh schedules" onClick={noop} />
        <Button onClick={noop}>
          <Plus />
          {scheduleCopy.newSchedule}
        </Button>
      </>
    ),
  },
  {
    id: 'workflows',
    title: 'Workflows',
    actions: (
      <>
        <Filter label="Filter workflows" />
        <PageHeaderAction icon={Upload} label="Import workflow" onClick={noop} />
        <Button onClick={noop}>
          <Plus />
          {workflowsCopy.newWorkflow}
        </Button>
      </>
    ),
  },
  {
    id: 'extensions',
    title: 'Extensions',
    actions: (
      <>
        <Filter label="Filter extensions" />
        <AddExtensionMenu onBrowse={noop} onFromFile={noop} onCustom={noop} />
      </>
    ),
  },
  {
    id: 'skills',
    title: 'Skills',
    actions: (
      <>
        <Filter label="Filter skills" />
        <AddMenu label={SKILLS_COPY.add} />
      </>
    ),
  },
  {
    id: 'knowledge',
    title: 'Knowledge',
    actions: (
      <>
        <PageHeaderAction icon={ClipboardList} label="Check for problems" onClick={noop} />
        <PageHeaderAction icon={RefreshCw} label="Refresh graph" onClick={noop} />
        <PageHeaderMenu label="More knowledge base actions" items={menu} />
      </>
    ),
  },
  {
    id: 'dictionary',
    title: 'Dictionary',
    actions: (
      <>
        <Filter label={DICTIONARY_LABELS.filter} />
        <Button variant="ghost" onClick={noop}>
          {DICTIONARY_LABELS.export}
        </Button>
        <AddMenu label={DICTIONARY_LABELS.add} />
        <PageHeaderMenu label={DICTIONARY_LABELS.pageMenu} items={menu} />
      </>
    ),
  },
  {
    id: 'applications',
    title: 'Applications',
    actions: (
      <>
        <Filter label="Filter apps" />
        <PageHeaderAction icon={RefreshCw} label="Refresh" onClick={noop} />
      </>
    ),
  },
  {
    id: 'history',
    title: 'Chat history',
    actions: (
      <>
        <Filter label="Search history" className="br-history-filter" />
        <PageHeaderAction icon={Upload} label="Show subagent runs" aria-pressed={false} onClick={noop} />
        <PageHeaderAction icon={Upload} label="Import chat" onClick={noop} />
      </>
    ),
  },
  {
    id: 'schedule-detail',
    title: 'nightly-cohort',
    back: true,
    actions: (
      <>
        <PageHeaderAction icon={Pause} label={scheduleCopy.pause} onClick={noop} />
        <PageHeaderMenu items={menu} />
        <Button onClick={noop}>{scheduleCopy.runNow}</Button>
      </>
    ),
  },
  {
    id: 'saved-chat',
    title: 'A saved chat',
    actions: (
      <>
        <PageHeaderAction icon={Share2} label="Share" onClick={noop} />
        <Button variant="secondary" shape="round" aria-label="Resume" onClick={noop}>
          <ArrowRight aria-hidden="true" />
        </Button>
      </>
    ),
  },
];

function App() {
  return (
    <div style={{ width: 1200 }}>
      {PAGES.map((page) => (
        <div key={page.id} id={page.id} style={{ display: 'flex', flexDirection: 'column' }}>
          <PageHeader
            title={page.title}
            info="Help."
            onBack={page.back ? noop : undefined}
            actions={page.actions}
          />
        </div>
      ))}
      {/* The same primary outside any band: the rule is the band's, not the button's. */}
      <div style={{ padding: 16 }}>
        <Button id="outside-primary">
          <Plus />
          {workflowsCopy.newWorkflow}
        </Button>
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
`;

const GALLERY_IDS = [
  'scheduler',
  'workflows',
  'extensions',
  'skills',
  'knowledge',
  'dictionary',
  'applications',
  'history',
  'schedule-detail',
  'saved-chat',
];

/** The band's re-pointing of the `md` rung and its glyph gap, as written in main.css. */
const BAND_RUNG = /\.biorouter-page-header-bar \{\s*--control-md: var\(--control-sm\);\s*\}/;
const BAND_GAP = /\.biorouter-page-header-bar \[data-slot='button'\] \{\s*column-gap: 6px;\s*\}/;

type Control = {
  page: string;
  name: string;
  tag: string;
  labelled: boolean;
  h: number;
  w: number;
  mid: number;
  barMid: number;
  radius: string;
  padL: string;
  padR: string;
  gap: string;
  icon: [number, number] | null;
};

describe('every page band control is one size (owner, 2026-11-19)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the page band control size tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: GALLERY, loader: 'tsx', resolveDir: here, sourcefile: 'gallery.tsx' },
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
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(MAIN_CSS, { base: STYLES, onDependency() {} });
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function openGallery(stylesheet: string): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1240, height: 900 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${stylesheet}</style></head>` +
        `<body style="margin:0"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#outside-primary');
    return tab;
  }

  /** A length a probe paints for `value`, so the expectations follow the tokens. */
  const probe = (tab: Page, property: 'height' | 'borderTopLeftRadius', value: string) =>
    tab.evaluate(
      ([p, v]) => {
        const el = document.createElement('div');
        (el.style as unknown as Record<string, string>)[p] = v;
        document.body.append(el);
        const out = getComputedStyle(el)[p as 'height'];
        el.remove();
        return out;
      },
      [property, value] as const
    );

  /** Every control in every band's bar, less the title's InfoTip (help, not an action). */
  const controls = (tab: Page) =>
    tab.evaluate((ids) => {
      const out: Control[] = [];
      for (const id of ids) {
        const bar = document.querySelector(`#${id} .biorouter-page-header-bar`)!;
        const b = bar.getBoundingClientRect();
        for (const el of Array.from(bar.querySelectorAll('button, input'))) {
          if (el.getAttribute('aria-label')?.startsWith('About ')) continue;
          const r = el.getBoundingClientRect();
          const cs = getComputedStyle(el);
          const svg = el.tagName === 'BUTTON' ? el.querySelector('svg') : null;
          const sr = svg?.getBoundingClientRect();
          out.push({
            page: id,
            name: el.getAttribute('aria-label') || (el.textContent ?? '').trim(),
            tag: el.tagName.toLowerCase(),
            labelled: el.tagName === 'BUTTON' && (el.textContent ?? '').trim().length > 0,
            h: r.height,
            w: r.width,
            mid: r.top + r.height / 2,
            barMid: b.top + b.height / 2,
            radius: cs.borderTopLeftRadius,
            padL: cs.paddingLeft,
            padR: cs.paddingRight,
            gap: cs.columnGap,
            icon: sr ? [sr.width, sr.height] : null,
          });
        }
      }
      return out;
    }, GALLERY_IDS);

  it('draws every control of every band at the sm rung, centred, with one radius', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openGallery(css);
    try {
      const sm = parseFloat(await probe(tab, 'height', 'var(--control-sm)'));
      const md = parseFloat(await probe(tab, 'height', 'var(--control-md)'));
      const radius = await probe(tab, 'borderTopLeftRadius', 'var(--radius-element)');
      expect(sm, 'the rungs differ, or the test proves nothing').toBeLessThan(md);
      const all = await controls(tab);
      // Every page contributes, and each kind of control the bands carry is here.
      expect(new Set(all.map((c) => c.page))).toEqual(new Set(GALLERY_IDS));
      expect(all.some((c) => c.tag === 'input')).toBe(true);
      expect(all.some((c) => c.labelled)).toBe(true);
      expect(all.some((c) => !c.labelled && c.tag === 'button')).toBe(true);
      for (const c of all) {
        const what = `${c.page} / ${c.name}`;
        near(c.h, sm, `${what} height`);
        expect(c.radius, `${what} radius`).toBe(radius);
        near(c.mid, c.barMid, `${what} vertical centre`);
        // An icon button is square: its width follows the same rung as its height.
        if (c.tag === 'button' && !c.labelled) near(c.w, sm, `${what} width`);
      }
      // One height and one radius across every page, not merely each within a tolerance.
      expect(new Set(all.map((c) => Math.round(c.h * 2) / 2)).size).toBe(1);
      expect(new Set(all.map((c) => c.radius)).size).toBe(1);
    } finally {
      await tab.close();
    }
  });

  it('gives every labelled band button the same padding, glyph and gap', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openGallery(css);
    try {
      const labelled = (await controls(tab)).filter((c) => c.labelled);
      // The primaries of five pages, the ghost Export… and Run now.
      expect(labelled.length).toBeGreaterThanOrEqual(7);
      for (const c of labelled) {
        const what = `${c.page} / ${c.name}`;
        expect(c.padL, `${what} leading padding`).toBe('12px');
        expect(c.padR, `${what} trailing padding`).toBe('12px');
        if (c.icon) {
          near(c.icon[0], 16, `${what} glyph width`);
          near(c.icon[1], 16, `${what} glyph height`);
          expect(c.gap, `${what} glyph-to-label gap`).toBe('6px');
        }
      }
      expect(labelled.filter((c) => c.icon).length).toBeGreaterThanOrEqual(5);
      // Every icon button's glyph is the same 16px.
      for (const c of (await controls(tab)).filter((x) => x.tag === 'button' && !x.labelled)) {
        expect(c.icon, `${c.page} / ${c.name} glyph`).not.toBeNull();
        near(c.icon![0], 16, `${c.page} / ${c.name} glyph width`);
      }
    } finally {
      await tab.close();
    }
  });

  it('leaves the same button at the md rung outside a band', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openGallery(css);
    try {
      const md = parseFloat(await probe(tab, 'height', 'var(--control-md)'));
      const outside = await tab.evaluate(() => {
        const el = document.getElementById('outside-primary')!;
        return { h: el.getBoundingClientRect().height, gap: getComputedStyle(el).columnGap };
      });
      near(outside.h, md, 'a primary outside the band');
      expect(outside.gap).toBe('8px');
    } finally {
      await tab.close();
    }
  });

  // The instrument: without the band's two declarations the same measurement reads what the owner
  // saw, a 32px primary and 32px icon buttons beside the 28px filter. A test that cannot fail
  // says nothing.
  it('measures two heights in one band when the band rule is removed', async (ctx) => {
    if (!browser) return ctx.skip();
    expect(css, 'the band re-points the md rung').toMatch(BAND_RUNG);
    expect(css, 'the band sets the sm glyph gap').toMatch(BAND_GAP);
    const tab = await openGallery(css.replace(BAND_RUNG, '').replace(BAND_GAP, ''));
    try {
      const sm = parseFloat(await probe(tab, 'height', 'var(--control-sm)'));
      const md = parseFloat(await probe(tab, 'height', 'var(--control-md)'));
      const all = await controls(tab);
      const primary = all.find((c) => c.page === 'extensions' && c.labelled)!;
      const filter = all.find((c) => c.page === 'extensions' && c.tag === 'input')!;
      near(primary.h, md, 'the old primary height');
      near(filter.h, sm, 'the filter, which was already sm');
      expect(new Set(all.map((c) => Math.round(c.h * 2) / 2)).size).toBeGreaterThan(1);
    } finally {
      await tab.close();
    }
  });
});
