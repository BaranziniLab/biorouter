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
import { Dropzone } from './IngestPanel/Dropzone';
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
.br-tabs-indicator { display: none; }
.px-4 { padding-inline: 16px; } .py-4 { padding-block: 16px; } .py-1 { padding-block: 4px; }
.text-center { text-align: center; } .mt-1 { margin-top: 4px; }
.h-8 { height: 32px; } .w-8 { width: 32px; } .h-control-sm { height: 28px; }
.rounded-container { border-radius: 12px; } .gap-1\\.5 { gap: 6px; } .px-3 { padding-inline: 12px; }
.cursor-pointer { cursor: pointer; }`;

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
    cut(main, '.br-knowledge-pane {', '/* @ws WS-KNOWLEDGE begin: filter facet */'),
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

/**
 * The modal surface's rule for a header-like direct child (`.biorouter-modal-surface > .border-b`):
 * a Sheet's content is a modal surface, so this is what a drawer's header meets in the app.
 */
function modalSurfaceCss(): string {
  const main = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
  return cut(main, '.biorouter-modal-surface > .border-b {', '}');
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
    <div class="biorouter-modal-surface br-knowledge-drawer" id="drawer" style="position:relative;display:flex;flex-direction:column;width:340px;height:300px">
      ${header}
      <button id="primitive-close" data-slot="sheet-close" type="button" style="position:absolute;top:16px;right:16px;width:32px;height:32px">x</button>
    </div>
  </body></html>`;
}

/**
 * The two workspace panels as `KnowledgeView` mounts them: the Tabs primitive's `animate-in fade-in`
 * (a keyframe in the utilities LAYER, as Tailwind emits it) on each, `hidden` or `flex` from React,
 * inside the pane container. `enter` is the keyframe `tw-animate-css` names; its opacity is all
 * that matters here, because the question is whether any animation exists.
 */
function panelsPage(css: string, paneWidth: number): string {
  return `<!doctype html><html><head><style>${TOKENS}
@keyframes enter { from { opacity: 0; } to { opacity: 1; } }
@layer utilities {
  .animate-in { animation: enter 175ms linear both; }
  .flex { display: flex; }
  .hidden { display: none; }
}
${css}</style></head><body>
    <div class="br-knowledge-pane" id="pane" style="width:${paneWidth}px;height:400px">
      <div class="br-knowledge-body" style="height:400px">
        <div id="sources" class="br-knowledge-sources br-tabs-content animate-in hidden"></div>
        <div id="graph" class="br-knowledge-graph br-tabs-content animate-in flex"></div>
      </div>
    </div>
  </body></html>`;
}

/** The dropzone block and the footer column, cut at their owner's markers. */
function ingestCss(): string {
  const main = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
  return cut(
    main,
    '/* @ws WS-KNOWLEDGE begin: dropzone */',
    '/* @ws WS-KNOWLEDGE end: dropzone */'
  );
}

/**
 * The Sources rail as `IngestPanel` lays it out: the real `Dropzone` at the top of the rail's
 * scroller (`flex-1`, `gap-3`, `p-4`, `overflow-y-auto`), the footer column
 * (`.br-knowledge-ingest-actions`) under it with a model field and a full-width Digest.
 *
 * `paste` and `staged` add stand-ins for what the box yields its room to (R-33): the paste box (a
 * wrapper around a 248px card, its height at 1440) and N staged rows of 40px. Each stand-in's
 * height is its CONTENT's, as the real ones' is, so it cannot shrink under the box the way an
 * empty div with a fixed height can (its automatic minimum is its content, 0).
 */
function railPage(
  css: string,
  railWidth: number,
  { paste = false, staged = 0, height = 760 }: RailOptions = {}
): string {
  const box = renderToStaticMarkup(
    <Dropzone
      onFiles={() => undefined}
      onPathPickRequested={() => undefined}
      onPasteText={() => undefined}
    />
  );
  const pasteBox = paste ? '<div id="paste"><div style="height:248px"></div></div>' : '';
  const stagedList = staged
    ? `<div id="staged">${'<div style="height:40px"></div>'.repeat(staged)}</div>`
    : '';
  return `<!doctype html><html><head><style>${TOKENS}
.hidden { display: none; }
${css}</style></head><body>
    <div id="rail" style="width:${railWidth}px;height:${height}px;display:flex;flex-direction:column">
      <div id="body" style="flex:1 1 0%;min-height:0;display:flex;flex-direction:column;justify-content:flex-start;gap:12px;padding:16px;overflow-y:auto">
        ${box}
        ${pasteBox}
        ${stagedList}
      </div>
      <div id="footer" style="flex:none;padding:16px">
        <div class="br-knowledge-ingest-actions">
          <div id="picker" style="height:32px"></div>
          <button id="digest" type="button" style="height:36px;width:100%">Digest staged sources</button>
        </div>
      </div>
    </div>
  </body></html>`;
}

type RailOptions = { paste?: boolean; staged?: number; height?: number };

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
      await tab.setContent(drawerPage(`${modalSurfaceCss()}\n${paneHeaderCss()}`));
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

  // V3-11: the drawer's header is the page band's ground (`--sidebar`), not the transparent
  // white `.biorouter-modal-surface > .border-b` paints a modal header.
  it('keeps the drawer header on the band ground the modal surface would clear', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await browser.newPage({ viewport: { width: 800, height: 600 } });
    try {
      await tab.setContent(drawerPage(`${modalSurfaceCss()}\n${paneHeaderCss()}`));
      const ground = await tab.evaluate(() => {
        const header = document.querySelector('.br-knowledge-pane-header')!;
        const probe = document.createElement('div');
        probe.style.background = 'var(--sidebar)';
        document.body.appendChild(probe);
        const sidebar = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return { header: getComputedStyle(header).backgroundColor, sidebar };
      });
      expect(ground.sidebar).not.toBe('rgba(0, 0, 0, 0)');
      expect(ground.header).toBe(ground.sidebar);
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

  // V3-01: dragging the window across the 860px step flips the Sources panel from `display: none`
  // to shown, and the browser restarts a panel's animation whenever its `display` leaves `none`.
  describe('the tab panels\u2019 enter animation', () => {
    const settle = (tab: Page) =>
      tab.evaluate(
        () =>
          new Promise<void>((done) =>
            requestAnimationFrame(() => requestAnimationFrame(() => done()))
          )
      );
    const running = (tab: Page, id: string) =>
      tab.evaluate(
        (panel) =>
          document
            .getElementById(panel)!
            .getAnimations()
            .map((a) => a.id || 'css'),
        id
      );
    const resize = (tab: Page, width: number) =>
      tab.evaluate((w) => {
        document.getElementById('pane')!.style.width = `${w}px`;
      }, width);

    it('plays on no panel while the pane crosses the step, in either direction', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await browser.newPage({ viewport: { width: 1600, height: 900 } });
      try {
        await tab.setContent(panelsPage(css, 760));
        await settle(tab);
        // Narrow to wide: the container forces Sources on.
        await resize(tab, 940);
        await settle(tab);
        expect(await visible(tab, '#sources')).toBe(true);
        expect(await running(tab, 'sources')).toEqual([]);
        expect(await running(tab, 'graph')).toEqual([]);
        // And back: the panel that stays shown must not restart either.
        await resize(tab, 760);
        await settle(tab);
        expect(await running(tab, 'sources')).toEqual([]);
        expect(await running(tab, 'graph')).toEqual([]);
      } finally {
        await tab.close();
      }
    });

    it('still plays for the panel a person just picked', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await browser.newPage({ viewport: { width: 1600, height: 900 } });
      try {
        await tab.setContent(panelsPage(css, 760));
        await settle(tab);
        // What React writes when the Sources tab is picked: shown, and marked as entering.
        await tab.evaluate(() => {
          const sources = document.getElementById('sources')!;
          sources.setAttribute('data-entering', '');
          sources.classList.replace('hidden', 'flex');
        });
        expect(await running(tab, 'sources')).toHaveLength(1);
        expect(await running(tab, 'graph')).toEqual([]);
      } finally {
        await tab.close();
      }
    });
  });
  // R-33 (over V3-12's height): the dropzone fills the rail's free height again and yields it to
  // what appears under it. V3-12's other half stands: Digest is a button, not a bar.
  describe('the Sources rail', () => {
    async function measure(width: number, opts: RailOptions = {}) {
      const tab = await browser!.newPage({ viewport: { width: 1600, height: 900 } });
      try {
        await tab.setContent(railPage(ingestCss(), width, opts));
        const scroller = await tab.evaluate(() => {
          const body = document.getElementById('body')!;
          const pad = parseFloat(getComputedStyle(body).paddingTop);
          // The height the scroller's children share: its client box less the 16px padding.
          return {
            free: body.clientHeight - 2 * pad,
            scrolls: body.scrollHeight > body.clientHeight + 1,
          };
        });
        // The box's content group, the label button down to Paste text (the file input is hidden).
        const content = await tab.evaluate(() => {
          const zone = document.querySelector('[data-testid="knowledge-ingest-dropzone"]')!;
          const shown = [...zone.children].filter((c) => c.getClientRects().length > 0);
          return {
            top: shown[0].getBoundingClientRect().top,
            bottom: shown[shown.length - 1].getBoundingClientRect().bottom,
          };
        });
        return {
          box: (await box(tab, '[data-testid="knowledge-ingest-dropzone"]'))!,
          digest: (await box(tab, '#digest'))!,
          picker: (await box(tab, '#picker'))!,
          content,
          ...scroller,
        };
      } finally {
        await tab.close();
      }
    }
    const heightOf = (b: Box) => b.bottom - b.top;

    it('fills the rail’s free height with the dropzone, like a side panel', async (ctx) => {
      if (!browser) return ctx.skip();
      // The 264px and 300px rails of the two-column steps, and the narrow Sources tab.
      for (const width of [264, 300, 820]) {
        const m = await measure(width);
        const height = heightOf(m.box);
        expect(Math.abs(height - m.free), `dropzone height at a ${width}px rail`).toBeLessThan(2);
        // V3-12's box was about 160px; the scroller here is 600px tall.
        expect(height, `dropzone height at a ${width}px rail`).toBeGreaterThan(500);
        expect(m.box.width).toBe(width - 32);
        expect(m.scrolls).toBe(false);
        // The plate, the two lines and Paste text sit in the middle of the box, as one group.
        const mid = (m.box.top + m.box.bottom) / 2;
        expect(Math.abs((m.content.top + m.content.bottom) / 2 - mid)).toBeLessThan(2);
      }
    });

    it('yields its room to the paste box and the staged list, and stays a generous target', async (ctx) => {
      if (!browser) return ctx.skip();
      const empty = heightOf((await measure(300)).box);

      // The paste box opens under it: the box gives back exactly the card and one 12px gap.
      const paste = await measure(300, { paste: true });
      expect(Math.abs(heightOf(paste.box) - (empty - 248 - 12))).toBeLessThan(2);
      expect(paste.scrolls).toBe(false);

      // The paste box and three staged rows together, in the rail a 900px window gives (856px):
      // no scroll, and still over the floor.
      const tall = heightOf((await measure(300, { height: 856 })).box);
      const both = await measure(300, { paste: true, staged: 3, height: 856 });
      expect(Math.abs(heightOf(both.box) - (tall - 248 - 12 - 120 - 12))).toBeLessThan(2);
      expect(heightOf(both.box)).toBeGreaterThan(240);
      expect(both.scrolls).toBe(false);

      // A long list: the box stops at its 240px floor and the scroller scrolls instead.
      const long = await measure(300, { staged: 30 });
      expect(Math.round(heightOf(long.box))).toBe(240);
      expect(long.scrolls).toBe(true);

      // A short window with the paste box open: the floor holds there too.
      const short = await measure(300, { paste: true, height: 520 });
      expect(Math.round(heightOf(short.box))).toBe(240);
      expect(short.scrolls).toBe(true);
    });

    it('caps the model field and Digest at one column, left with the dropzone', async (ctx) => {
      if (!browser) return ctx.skip();
      // The two-column step's rail: the cap does nothing and the column is the rail's width.
      const rail = await measure(288);
      expect(rail.digest.width).toBe(288 - 32);
      // The narrow Sources tab: the rail is the whole pane, and Digest stops at 360px.
      const wide = await measure(820);
      expect(wide.digest.width).toBeLessThanOrEqual(360);
      expect(wide.picker.width).toBe(wide.digest.width);
      expect(wide.digest.left).toBe(wide.box.left);
    });
  });
});
