// @vitest-environment node
/**
 * The Knowledge band, measured in a real layout engine (spec 3.12 acceptance, 5.7's pattern).
 *
 * jsdom lays nothing out and evaluates no `@container` query, so `KnowledgeView.test.tsx` can say
 * the tier control is in the band but not that it stays on screen at a narrow pane, that the base's
 * name gives way first, or that the Sources/Graph tabs appear only below the 860px step. This
 * renders the real `PageHeader`, `KbTierControl`, `Badge` and `Tabs` to markup and lays them out
 * in Chromium with the band and Knowledge CSS cut from `main.css` at their owners' markers, at the
 * token values. The subject switcher is a stand-in with the real trigger's box rules, because the
 * real one reads the Knowledge context.
 *
 * The second half measures the drawers' pane header: the same 44px band as a preview's, its close
 * 8px from the edge and centred in the band, and the Sheet primitive's own corner close hidden.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { PageHeader, PageHeaderAction } from '../Layout/PageHeader';
import { ClipboardList, MoreHorizontal, RefreshCw } from '../icons/app-icons';
import { Badge } from '../ui/badge';
import { Sheet } from '../ui/sheet';
import { Tabs, TabsList, TabsTrigger } from '../ui/tabs';
import { KbTierControl } from './KbTierControl';
import { KnowledgePaneHeader } from './KnowledgePaneHeader';
import { knowledgeCopy } from './copy';

const here = dirname(fileURLToPath(import.meta.url));

/** `main.css`'s values for the tokens the band reads, and the utilities its markup uses. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, sans-serif;
  --chrome-height: 44px;
  --sidebar: #f7f7f5;
  --border-subtle: #e4e4e0;
  --border-focus: #6b6963;
  --background-medium: #ecece9;
  --background-muted: #f4f4f2;
  --radius-inner: 4px;
  --dur-fast-min: 95ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
* { box-sizing: border-box; }
body { margin: 0; font: 14px/20px var(--font-body); }
h1 { margin: 0; font: inherit; }
button { font: inherit; padding: 0; border: 0; background: none; }
svg { width: 16px; height: 16px; flex-shrink: 0; }
.flex { display: flex; } .flex-col { flex-direction: column; } .inline-flex { display: inline-flex; }
.relative { position: relative; }
.items-center { align-items: center; } .justify-center { justify-content: center; }
.min-w-0 { min-width: 0; } .shrink-0, .flex-shrink-0 { flex-shrink: 0; } .flex-1 { flex: 1 1 0%; }
.gap-1 { gap: 4px; } .gap-2 { gap: 8px; } .gap-5 { gap: 20px; }
.px-1\\.5 { padding-inline: 6px; } .px-2 { padding-inline: 8px; } .p-0 { padding: 0; }
.h-5 { height: 20px; } .h-3 { height: 12px; } .w-3 { width: 12px; }
.h-chrome { height: var(--chrome-height); } .flex-none { flex: none; }
.border-b { border-bottom: 1px solid; } .border-border-subtle { border-color: var(--border-subtle); }
.bg-sidebar { background: var(--sidebar); } .rounded-element { border-radius: 8px; }
.h-control-md { height: 32px; } .w-control-md { width: 32px; } .h-control-lg { height: 36px; }
.max-w-\\[22rem\\] { max-width: 22rem; }
.whitespace-nowrap { white-space: nowrap; }
.truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
.text-supporting { font-size: 12px; line-height: 16px; }
.text-chip { font-size: 12px; line-height: 16px; font-weight: 500; }
.tabular-nums { font-variant-numeric: tabular-nums; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
.br-info-tip { display: inline-flex; width: 24px; height: 24px; margin: -5px; align-items: center;
  justify-content: center; }
.br-tabs-indicator { display: none; }`;

/** A rule block of `main.css`, cut between two markers (inclusive of everything between). */
function cut(main: string, begin: string, end: string): string {
  const from = main.indexOf(begin);
  const to = main.indexOf(end, from);
  if (from < 0 || to < from) throw new Error(`markers not found in main.css: ${begin}`);
  return main.slice(from, to + end.length);
}

function bandCss(): string {
  const main = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
  return [
    // The page band (WS-VIEWS-A), its hairline rule just above it.
    cut(main, '.biorouter-page-header {', '/* @ws WS-VIEWS-A end: page band */'),
    // The Knowledge pane: the container, the step defaults, the band block, the ladder.
    cut(main, '.br-knowledge-pane {', '/* ── the filter control (R-02)'),
  ].join('\n');
}

/** The pane header's rules, cut at their owner's markers. */
function paneHeaderCss(): string {
  const main = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
  return cut(
    main,
    '/* @ws WS-KNOWLEDGE begin: pane header */',
    '/* @ws WS-KNOWLEDGE end: pane header */'
  );
}

const LONG_NAME = 'A knowledge base whose name runs on far past the width a band could give it';

/** The band as `KnowledgeView` renders it, inside the pane container, over a two-column body. */
function page(css: string, paneWidth: number, name: string): string {
  const band = renderToStaticMarkup(
    <Tabs value="graph">
      <PageHeader
        title={knowledgeCopy.title}
        info={knowledgeCopy.viewInfo}
        adornment={
          <>
            <span aria-hidden="true">/</span>
            {/* The real trigger's box rules (`KBSelectorTrigger` variant="subject"). */}
            <button
              type="button"
              id="subject"
              className="flex h-control-md min-w-0 max-w-[22rem] items-center gap-2 px-2 text-label"
            >
              <span className="h-3 w-3 shrink-0" />
              <span id="subject-name" className="min-w-0 flex-1 truncate">
                {name}
              </span>
              <svg aria-hidden="true" />
            </button>
            <Badge id="format">OKF</Badge>
            <KbTierControl
              kb={{ id: 'omop', name: 'OMOP', tier: 'private' }}
              onSetTier={() => undefined}
            />
            <span id="summary" className="shrink-0">
              {knowledgeCopy.summary(43, 38)}
            </span>
          </>
        }
        tabs={
          <TabsList className="br-knowledge-tabs border-b-0">
            <TabsTrigger value="digest">{knowledgeCopy.tabs.sources}</TabsTrigger>
            <TabsTrigger value="graph">{knowledgeCopy.tabs.graph}</TabsTrigger>
          </TabsList>
        }
        actions={
          <>
            <PageHeaderAction icon={ClipboardList} label={knowledgeCopy.actions.check} />
            <PageHeaderAction icon={RefreshCw} label={knowledgeCopy.actions.refresh} />
            <PageHeaderAction icon={MoreHorizontal} label={knowledgeCopy.actions.more} />
          </>
        }
      />
    </Tabs>
  );
  return `<!doctype html><html><head><style>${TOKENS}\n${css}</style></head><body>
    <div class="br-knowledge-pane" id="pane" style="width:${paneWidth}px;height:700px;display:flex;flex-direction:column">
      ${band}
      <div class="br-knowledge-body" style="flex:1 1 auto">
        <div class="br-knowledge-sources" id="sources" style="display:flex"></div>
        <div class="br-knowledge-graph" id="graph"></div>
      </div>
    </div>
  </body></html>`;
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

/**
 * A drawer as `ChangeLogDrawer` builds it: the pane header with its own close, then the Sheet
 * primitive's corner close, a direct child of the content named `data-slot="sheet-close"`
 * (`ui/sheet.tsx`: absolute, top and right 16px, 32px square). The primitive's is the one
 * `.br-knowledge-drawer` must hide; the header's own `SheetClose` sits one level down and must
 * stay.
 */
function drawerPage(css: string): string {
  const header = renderToStaticMarkup(
    <Sheet open>
      <KnowledgePaneHeader
        sheet
        closeLabel={knowledgeCopy.drawers.closeChangeLog}
        title={<h2 className="min-w-0 truncate text-label">{knowledgeCopy.drawers.changeLog}</h2>}
      />
    </Sheet>
  );
  return `<!doctype html><html><head><style>${TOKENS}\n${css}</style></head><body>
    <div class="br-knowledge-drawer" id="drawer" style="position:relative;display:flex;flex-direction:column;width:340px;height:300px">
      ${header}
      <button id="primitive-close" data-slot="sheet-close" type="button" style="position:absolute;top:16px;right:16px;width:32px;height:32px">x</button>
    </div>
  </body></html>`;
}

type Box = { left: number; right: number; top: number; bottom: number; width: number };

describe('the Knowledge band in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the Knowledge band ' +
          'geometry tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = bandCss();
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(paneWidth: number, name = 'OMOP'): Promise<Page> {
    const tab = await browser!.newPage({
      viewport: { width: 1600, height: 900 },
      reducedMotion: 'reduce',
    });
    await tab.setContent(page(css, paneWidth, name));
    return tab;
  }

  const box = (tab: Page, selector: string) =>
    tab.evaluate((sel) => {
      const el = document.querySelector(sel);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width };
    }, selector) as Promise<Box | null>;

  const visible = (tab: Page, selector: string) =>
    tab.evaluate((sel) => {
      const el = document.querySelector(sel) as HTMLElement | null;
      return !!el && getComputedStyle(el).display !== 'none' && el.getClientRects().length > 0;
    }, selector);

  // The three acceptance widths: a 1440 window's pane, an 1100 window's, and a pane under 860.
  for (const width of [1152, 812, 760]) {
    it(`is one 44px band with the tier badge on screen at a ${width}px pane`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(width, LONG_NAME);
      try {
        const band = (await box(tab, '[data-testid="page-header"]'))!;
        expect(band.bottom - band.top).toBe(44);

        const tier = (await box(tab, '[data-testid="kb-tier-chip"]'))!;
        const actions = (await box(tab, '.biorouter-page-header-actions'))!;
        expect(tier.width).toBeGreaterThan(0);
        expect(tier.top).toBeGreaterThanOrEqual(band.top);
        expect(tier.bottom).toBeLessThanOrEqual(band.bottom);
        // Entirely before the actions: nothing overlaps, nothing is pushed off the pane.
        expect(tier.right).toBeLessThanOrEqual(actions.left);
        expect(actions.right).toBeLessThanOrEqual(band.right);

        // The base's NAME is what gives way, never the page's title or the tabs: without the
        // adornment's shrink rule the band kept the name whole and truncated "Knowledge" to
        // nothing instead.
        const clipped = await tab.evaluate(() => {
          const over = (el: Element | null) => !!el && el.scrollWidth > el.clientWidth;
          return {
            name: over(document.getElementById('subject-name')),
            title: over(document.querySelector('[data-testid="page-header"] h1')),
            tabs: over(document.querySelector('.biorouter-page-header-tabs')),
          };
        });
        expect(clipped).toEqual({ name: true, title: false, tabs: false });
      } finally {
        await tab.close();
      }
    });
  }

  // The group break the merged band had dropped: with the band's 8px between children the pair
  // began right after the counts and the row read as "38 links Sources Graph".
  it('sets the Sources/Graph pair apart from the counts, by more than the tabs are apart', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const width of [760, 812]) {
      const tab = await open(width);
      try {
        const summary = (await box(tab, '#summary'))!;
        const tabs = await tab.evaluate(() =>
          [...document.querySelectorAll('.br-knowledge-tabs [role="tab"]')].map((el) => {
            const r = el.getBoundingClientRect();
            return { left: r.left, right: r.right };
          })
        );
        expect(tabs).toHaveLength(2);
        const breakBefore = tabs[0].left - summary.right;
        const betweenTabs = tabs[1].left - tabs[0].right;
        expect(breakBefore, `${width}px`).toBeGreaterThanOrEqual(24);
        expect(breakBefore, `${width}px`).toBeGreaterThan(betweenTabs);
      } finally {
        await tab.close();
      }
    }
  });

  it('shows the Sources/Graph tabs only below the 860px step', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const [width, shown] of [
      [760, true],
      [859, true],
      [860, false],
      [1152, false],
    ] as const) {
      const tab = await open(width);
      try {
        expect(await visible(tab, '.br-knowledge-tabs'), `${width}px`).toBe(shown);
      } finally {
        await tab.close();
      }
    }
  });

  it('draws a drawer’s header as the preview’s band: the close centred and 8px from the edge, the title 16px in', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await browser.newPage({ viewport: { width: 800, height: 600 } });
    try {
      await tab.setContent(drawerPage(paneHeaderCss()));
      const header = (await box(tab, '.br-knowledge-pane-header'))!;
      const close = (await box(
        tab,
        `button[aria-label="${knowledgeCopy.drawers.closeChangeLog}"]`
      ))!;
      const title = (await box(tab, '.br-knowledge-pane-header h2'))!;
      expect(header.bottom - header.top).toBe(44);
      // Centred in the band (the band's own hairline takes its last pixel), and 8px from the edge.
      const centre = (close.top + close.bottom) / 2;
      expect(Math.abs(centre - (header.top + 22))).toBeLessThanOrEqual(1);
      expect(header.right - close.right).toBe(8);
      // Text starts 16px in: 8px of band padding plus the title's own 8px.
      expect(title.left - header.left).toBe(16);
    } finally {
      await tab.close();
    }
  });

  it('hides the Sheet primitive’s corner close, so a drawer has exactly one', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await browser.newPage({ viewport: { width: 800, height: 600 } });
    try {
      await tab.setContent(drawerPage(paneHeaderCss()));
      expect(await visible(tab, '#primitive-close')).toBe(false);
      expect(
        await visible(tab, `button[aria-label="${knowledgeCopy.drawers.closeChangeLog}"]`)
      ).toBe(true);
    } finally {
      await tab.close();
    }
  });

  it('divides the two columns by an inset hairline from the 860px step, and only then', async (ctx) => {
    if (!browser) return ctx.skip();
    const shadow = async (width: number) => {
      const tab = await open(width);
      try {
        return await tab.evaluate(
          () => getComputedStyle(document.getElementById('sources')!).boxShadow
        );
      } finally {
        await tab.close();
      }
    };
    expect(await shadow(760)).toBe('none');
    expect(await shadow(940)).toMatch(/inset/);
  });
});
