// @vitest-environment node
/**
 * The window's floor (`WINDOW_MIN_WIDTH`, 721px) against the widest page band, in a real layout
 * engine (owner, 2026-11-19).
 *
 * Below rung 1 the sidebar is folded, and every band carries the titlebar reserve (macOS: the
 * traffic lights and the two floating controls, 172px) as a left margin. The Knowledge band is
 * the widest one: its title, the base (the switcher, its format and its tier), the counts, the
 * Sources/Graph pair and three actions. Measured in the running app before this change, its
 * counts painted over the Sources/Graph pair below a ~752px window, so a lower floor needed
 * the counts to fold (`main.css`, "THE WINDOW FLOOR, KNOWLEDGE'S PART").
 *
 * The real `PageHeader`, `Badge`, `KbTierControl` and `Tabs` are rendered to markup with the
 * widest labels any base can show (BioOKF, Private with its padlock, four-digit counts, a long
 * name), the whole `main.css` is compiled by Tailwind with the classes that markup uses, and
 * Chromium lays it out under the collapsed-sidebar selector the reserve keys on.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { ClipboardList, MoreHorizontal, RefreshCw } from '../icons/app-icons';
import { KbTierControl } from '../knowledge/KbTierControl';
import { knowledgeCopy } from '../knowledge/copy';
import { Badge } from '../ui/badge';
import { Tabs, TabsList, TabsTrigger } from '../ui/tabs';
import { PageHeader, PageHeaderAction } from './PageHeader';
import { getTitlebarControlReserve } from './TitlebarControls';
import { CHAT_MEASURE, WINDOW_MIN_WIDTH } from './yieldLadder';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const LONG_NAME = 'A knowledge base whose name runs on far past the width a band could give it';

/** The band as `KnowledgeView` renders it, at its widest. */
const BAND = renderToStaticMarkup(
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
              {LONG_NAME}
            </span>
            <svg aria-hidden="true" />
          </button>
          <Badge data-testid="knowledge-format-badge">BioOKF</Badge>
          <KbTierControl
            kb={{ id: 'omop', name: 'OMOP', tier: 'private' }}
            onSetTier={() => undefined}
          />
          <span data-testid="knowledge-graph-summary" className="shrink-0">
            {knowledgeCopy.summary(1204, 3881)}
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

describe('the window floor against the widest page band', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the window floor tests.');
      return;
    }
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    css = compiler.build(new Scanner({}).scanFiles([{ content: BAND, extension: 'html' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** The app shell at `width`, the sidebar folded, the band inside the Knowledge pane. */
  async function open(width: number, { docked = false } = {}): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width, height: 400 } });
    await tab.setContent(
      `<!doctype html><html class="light"><head><style>${css}</style></head><body style="margin:0">
        <div style="display:flex;--biorouter-titlebar-control-reserve:${getTitlebarControlReserve(true)}px">
          <div data-slot="sidebar" data-state="${docked ? 'expanded' : 'collapsed'}"></div>
          <div data-slot="sidebar-inset" style="flex:1 1 auto;min-width:0">
            <div class="br-knowledge-pane" id="pane" style="display:flex;flex-direction:column;height:400px">
              ${BAND}
            </div>
          </div>
        </div>
      </body></html>`
    );
    return tab;
  }

  const read = (tab: Page) =>
    tab.evaluate(() => {
      const bar = document.querySelector('.biorouter-page-header-bar')!;
      const box = (el: Element) => {
        const r = el.getBoundingClientRect();
        return { left: r.left, right: r.right, width: r.width };
      };
      const shown = (el: Element) =>
        getComputedStyle(el).display !== 'none' && el.getClientRects().length > 0;
      const adornment = document.querySelector('.biorouter-page-header-adornment')!;
      // Every visible piece of the band, in order: what must not overlap or overflow.
      const pieces = [
        document.querySelector('.biorouter-page-header-title')!,
        ...[...adornment.children].filter(shown),
        document.querySelector('.biorouter-page-header-tabs')!,
        document.querySelector('.biorouter-page-header-actions')!,
      ].map(box);
      return {
        bar: box(bar),
        pieces,
        adornmentOverflow: adornment.scrollWidth - adornment.clientWidth,
        titleClipped: (() => {
          const h1 = bar.querySelector('h1')!;
          return h1.scrollWidth > h1.clientWidth;
        })(),
        summaryShown: shown(document.querySelector('[data-testid="knowledge-graph-summary"]')!),
        tierShown: shown(document.querySelector('[data-testid="kb-tier-chip"]')!),
        formatShown: shown(document.querySelector('[data-testid="knowledge-format-badge"]')!),
      };
    });

  it('names the counts the way KnowledgeView does, so the fold reaches the real ones', () => {
    const view = readFileSync(resolve(here, '../knowledge/KnowledgeView.tsx'), 'utf8');
    expect(view).toContain('data-testid="knowledge-graph-summary"');
  });

  it('fits the Knowledge band at the floor: the counts fold, nothing overlaps or overflows', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(WINDOW_MIN_WIDTH);
    try {
      const m = await read(tab);
      expect(m.bar.left, 'the titlebar reserve is in force').toBe(getTitlebarControlReserve(true));
      expect(m.summaryShown).toBe(false);
      expect(m.tierShown).toBe(true);
      expect(m.formatShown).toBe(true);
      expect(m.titleClipped).toBe(false);
      expect(m.adornmentOverflow).toBeLessThanOrEqual(0);
      for (let i = 1; i < m.pieces.length; i++) {
        expect(m.pieces[i].left, `piece ${i} starts after piece ${i - 1}`).toBeGreaterThanOrEqual(
          m.pieces[i - 1].right - 0.5
        );
      }
      expect(m.pieces[m.pieces.length - 1].right).toBeLessThanOrEqual(m.bar.right + 0.5);
    } finally {
      await tab.close();
    }
  });

  /**
   * From the 860px step up the band has no Sources/Graph pair and room for the counts again,
   * and with the sidebar docked there is no reserve: the counts stay in both.
   */
  it('folds the counts only while the reserve and the pair share the band', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const [width, shown] of [
      [859, false],
      [860, true],
    ] as const) {
      const tab = await open(width);
      try {
        const m = await read(tab);
        expect(m.summaryShown, `${width}px`).toBe(shown);
        expect(m.adornmentOverflow, `${width}px`).toBeLessThanOrEqual(0);
      } finally {
        await tab.close();
      }
    }
    const docked = await open(CHAT_MEASURE, { docked: true });
    try {
      const m = await read(docked);
      expect(m.bar.left, 'no reserve beside a docked sidebar').toBeLessThan(
        getTitlebarControlReserve(true)
      );
      expect(m.summaryShown).toBe(true);
      expect(m.adornmentOverflow).toBeLessThanOrEqual(0);
    } finally {
      await docked.close();
    }
  });
});
