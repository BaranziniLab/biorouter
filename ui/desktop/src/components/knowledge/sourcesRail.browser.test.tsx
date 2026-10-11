// @vitest-environment node
/**
 * The Sources rail as the app builds it, laid out in Chromium (owner message 17, R-33).
 *
 * The drop zone fills the rail's free height like a side panel, yields that room to what appears
 * under it (the paste box, the staged list) down to a 240px floor, and the rail's scroller scrolls
 * below the floor. The zone's size is authored in `main.css`, but whether it HAS free height to
 * fill is `IngestPanel`'s layout: the scroller is the rail's one flex-1 child above a footer that is
 * its sibling, the zone is the scroller's one grower, and the scroller starts its content at the
 * top. A stand-in scroller written into the test said none of that, so a change to `IngestPanel`
 * could break the rail while this stayed green.
 *
 * So the real `SourcesRail` (and the real `IngestPanel`, `Dropzone`, `PasteTextBox`, `StagedList`
 * and model picker under it) is bundled with esbuild, mounted in the Knowledge pane's own column
 * classes, `styles/main.css` is compiled by Tailwind with the classes the bundle uses, and Chromium
 * lays it out. Only the three contexts the panel reads are stubbed (the base, the app's model, the
 * provider list). Sources are staged the way a person stages them, by dropping files on the rail,
 * and the paste box is summoned by pressing "Paste text".
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

/**
 * The column `KnowledgeView` mounts the rail in: the pane container (its width decides the
 * column's), the body grid, and the Sources panel with the classes the view gives it when the
 * Sources tab is picked. The graph panel is an empty sibling, so the two-column steps have their
 * second column.
 */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { SourcesRail } from './SourcesRail';

createRoot(document.getElementById('root')!).render(
  <MemoryRouter>
    <div className="br-knowledge-pane relative flex min-w-0 flex-1 flex-col overflow-hidden">
      <div className="br-knowledge-body min-h-0 flex-1">
        <div className="br-knowledge-sources flex mt-0 min-h-0 flex-col" id="sources">
          <SourcesRail className="flex-1" />
        </div>
        <div className="br-knowledge-graph hidden mt-0 min-h-0 min-w-0" />
      </div>
    </div>
  </MemoryRouter>
);
`;

/** What the panel reads from the app: a base with its own model, the app's model, no providers. */
const STUBS: Record<string, string> = {
  KnowledgeContext: `
    const value = {
      primaryKbId: 'kb-1',
      primaryKb: {
        id: 'kb-1',
        name: 'Notes',
        default_model: { provider: 'anthropic', model: 'claude-sonnet-4' },
      },
      loading: false,
      basesError: null,
      refresh: async () => {},
      triggerGraphRefresh: () => {},
    };
    export const useKnowledge = () => value;
  `,
  ModelAndProviderContext: `
    const value = {
      currentProvider: 'anthropic',
      currentModel: 'claude-sonnet-4',
      modelConfigStatus: 'ready',
    };
    export const useModelAndProvider = () => value;
  `,
  ConfigContext: `
    const value = { getProviders: async () => [], getProviderModels: async () => [] };
    export const useConfig = () => value;
  `,
};

const stubPlugin: Plugin = {
  name: 'sources-rail-stubs',
  setup(b) {
    b.onResolve(
      { filter: /\/(KnowledgeContext|ModelAndProviderContext|ConfigContext)$/ },
      (args) => {
        const name = Object.keys(STUBS).find((key) => args.path.endsWith(`/${key}`));
        return name ? { path: name, namespace: 'stub' } : undefined;
      }
    );
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: STUBS[args.path],
      loader: 'tsx',
      resolveDir: here,
    }));
  },
};

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

type Box = {
  left: number;
  right: number;
  top: number;
  bottom: number;
  width: number;
  height: number;
};

type Options = {
  /** The pane's height, which is the rail's (the band is not mounted). */
  height?: number;
  /** How many files to drop on the rail. */
  staged?: number;
  /** Press "Paste text". */
  paste?: boolean;
};

type Measure = {
  rail: Box;
  scroller: Box;
  footer: Box;
  zone: Box;
  digest: Box;
  picker: Box;
  /** The height the scroller's children share: its client box less its padding. */
  free: number;
  padTop: number;
  gap: number;
  scrolls: boolean;
  /** The zone's content group, the label button down to Paste text. */
  content: { top: number; bottom: number };
  /** The scroller's other children: each one's height, and its height at `flex: none`. */
  others: { height: number; natural: number }[];
};

describe('the Sources rail, the real tree in Chromium (R-33)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Sources rail tests.');
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
      plugins: [stubPlugin],
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** The rail in a pane `paneWidth` wide, with `staged` files dropped and the paste box open. */
  async function open(
    paneWidth: number,
    { height = 760, staged = 0, paste = false }: Options = {}
  ) {
    const tab = await browser!.newPage({
      viewport: { width: 1600, height: 1000 },
      reducedMotion: 'reduce',
    });
    const errors: string[] = [];
    tab.on('pageerror', (error) => errors.push(error.message));
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)">` +
        `<div style="display:flex;flex-direction:column;width:${paneWidth}px;height:${height}px">` +
        `<div id="root" style="display:flex;flex:1 1 0%;min-height:0;flex-direction:column"></div>` +
        `</div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-testid="knowledge-ingest-rail"]');
    if (staged > 0) {
      await tab.evaluate((n) => {
        const rail = document.querySelector('[data-testid="knowledge-ingest-rail"]')!;
        const files = new DataTransfer();
        for (let i = 1; i <= n; i++) {
          files.items.add(new File(['notes'], `source-${i}.md`, { type: 'text/markdown' }));
        }
        rail.dispatchEvent(
          new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: files })
        );
      }, staged);
      await tab.waitForFunction(
        (n) => document.querySelectorAll('[data-testid="knowledge-staged-item"]').length === n,
        staged
      );
    }
    if (paste) {
      await tab.click('[data-testid="knowledge-ingest-paste-text"]');
      await tab.waitForSelector('textarea');
    }
    await tab.waitForTimeout(50);
    expect(errors, 'page errors').toEqual([]);
    return tab;
  }

  const measure = (tab: Page): Promise<Measure> =>
    tab.evaluate(() => {
      const rect = (el: Element) => {
        const r = el.getBoundingClientRect();
        return {
          left: r.left,
          right: r.right,
          top: r.top,
          bottom: r.bottom,
          width: r.width,
          height: r.height,
        };
      };
      const q = (s: string) => document.querySelector(s)!;
      const rail = q('[data-testid="knowledge-ingest-rail"]');
      const zone = q('[data-testid="knowledge-ingest-dropzone"]');
      const scroller = zone.parentElement!;
      const footer = rail.lastElementChild!;
      const style = getComputedStyle(scroller);
      const padTop = parseFloat(style.paddingTop);
      const shown = [...zone.children].filter((c) => c.getClientRects().length > 0);
      // Every other child keeps its own height: measured as laid out, then at `flex: none`.
      const others = [...scroller.children]
        .filter((c): c is HTMLElement => c !== zone && c instanceof HTMLElement)
        .map((c) => {
          const height = c.getBoundingClientRect().height;
          const flex = c.style.flex;
          c.style.flex = 'none';
          const natural = c.getBoundingClientRect().height;
          c.style.flex = flex;
          return { height, natural };
        });
      return {
        rail: rect(rail),
        scroller: rect(scroller),
        footer: rect(footer),
        zone: rect(zone),
        digest: rect(q('[data-testid="knowledge-digest-button"]')),
        picker: rect(q('[data-testid="knowledge-model-picker-trigger"]')),
        free: scroller.clientHeight - padTop - parseFloat(style.paddingBottom),
        padTop,
        gap: parseFloat(style.rowGap),
        scrolls: scroller.scrollHeight > scroller.clientHeight + 1,
        content: {
          top: shown[0].getBoundingClientRect().top,
          bottom: shown[shown.length - 1].getBoundingClientRect().bottom,
        },
        others,
      };
    });

  async function measured(paneWidth: number, options: Options = {}): Promise<Measure> {
    const tab = await open(paneWidth, options);
    try {
      return await measure(tab);
    } finally {
      await tab.close();
    }
  }

  /** The scroller fills the rail above the footer, and the footer sits at the rail's foot. */
  function expectFramed(m: Measure, label: string) {
    expect(m.scroller.top, `${label}: scroller top`).toBeCloseTo(m.rail.top, 0);
    expect(m.scroller.bottom, `${label}: scroller meets the footer`).toBeCloseTo(m.footer.top, 0);
    expect(m.footer.bottom, `${label}: footer at the rail's foot`).toBeCloseTo(m.rail.bottom, 0);
  }

  /** The height the scroller's other children take, each with the gap before it. */
  const given = (m: Measure) => m.others.reduce((sum, o) => sum + o.height + m.gap, 0);
  /** What the zone gave back between two states of one rail: what appeared under it. */
  const yielded = (before: Measure, after: Measure) => given(after) - given(before);

  /**
   * Nothing but the zone grows or shrinks: every other child is its natural height, and the zone
   * takes exactly what they leave.
   */
  function expectOneGrower(m: Measure, label: string) {
    for (const [i, other] of m.others.entries()) {
      expect(
        Math.abs(other.height - other.natural),
        `${label}: child ${i + 2} keeps its height`
      ).toBeLessThan(1);
    }
    // Once the zone is at its floor the children overflow and the scroller scrolls instead.
    if (!m.scrolls) {
      expect(
        Math.abs(m.zone.height + given(m) - m.free),
        `${label}: the zone takes the rest`
      ).toBeLessThan(2);
    }
  }

  // A 900px pane gives the 264px column, an 1152px pane the 300px one, and an 812px pane is the
  // narrow step, where the Sources tab is the whole pane.
  const COLUMNS = [
    { pane: 900, column: 264 },
    { pane: 1152, column: 300 },
    { pane: 812, column: 812 },
  ];

  it('fills the rail’s free height with the dropzone, like a side panel', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const { pane, column } of COLUMNS) {
      const m = await measured(pane);
      const label = `${pane}px pane`;
      expect(m.rail.width, `${label}: the column`).toBe(column);
      expectFramed(m, label);
      expectOneGrower(m, label);
      expect(Math.abs(m.zone.height - m.free), `${label}: dropzone height`).toBeLessThan(2);
      // V3-12's box was about 160px; this rail's scroller is over 600px tall.
      expect(m.zone.height, `${label}: dropzone height`).toBeGreaterThan(500);
      expect(m.zone.width, `${label}: dropzone width`).toBe(column - 32);
      expect(m.zone.top - m.scroller.top, `${label}: dropzone at the top`).toBeCloseTo(m.padTop, 0);
      expect(m.scrolls, `${label}: scrolls`).toBe(false);
      // The plate, the two lines and Paste text sit in the middle of the box, as one group.
      const mid = (m.zone.top + m.zone.bottom) / 2;
      expect(Math.abs((m.content.top + m.content.bottom) / 2 - mid), label).toBeLessThan(2);
    }
  });

  it('yields its room to the paste box and the staged list, and stays a generous target', async (ctx) => {
    if (!browser) return ctx.skip();
    const empty = await measured(1152);

    // The paste box opens under it: the box gives back exactly the box and one gap.
    const paste = await measured(1152, { paste: true });
    expectFramed(paste, 'paste');
    expectOneGrower(paste, 'paste');
    expect(given(paste) - given(empty), 'the paste box and its gap').toBeGreaterThan(200);
    expect(Math.abs(paste.zone.height - (empty.zone.height - yielded(empty, paste)))).toBeLessThan(
      2
    );
    expect(paste.scrolls).toBe(false);

    // The paste box and three staged rows together, in the rail a 900px window gives (856px): no
    // scroll, and still over the floor.
    const tall = await measured(1152, { height: 856 });
    const both = await measured(1152, { paste: true, staged: 3, height: 856 });
    expectFramed(both, 'paste and three rows');
    expectOneGrower(both, 'paste and three rows');
    expect(Math.abs(both.zone.height - (tall.zone.height - yielded(tall, both)))).toBeLessThan(2);
    expect(both.zone.height).toBeGreaterThan(240);
    expect(both.scrolls).toBe(false);

    // A long list: the box stops at its 240px floor and the scroller scrolls instead, starting at
    // the box (an overflow that started above the scroller could never be scrolled to).
    const long = await measured(1152, { staged: 30 });
    expectFramed(long, 'thirty rows');
    expectOneGrower(long, 'thirty rows');
    expect(Math.round(long.zone.height)).toBe(240);
    expect(long.scrolls).toBe(true);
    expect(long.zone.top - long.scroller.top).toBeCloseTo(long.padTop, 0);

    // A short window with the paste box open: the floor holds there too.
    const short = await measured(1152, { paste: true, height: 520 });
    expectFramed(short, 'short window');
    expect(Math.round(short.zone.height)).toBe(240);
    expect(short.scrolls).toBe(true);
  });

  it('caps the model field and Digest at one column, left with the dropzone', async (ctx) => {
    if (!browser) return ctx.skip();
    // The two-column step's rail: the cap does nothing and the column is the rail's width.
    const rail = await measured(900);
    expect(rail.digest.width).toBe(264 - 32);
    expect(rail.picker.width).toBe(rail.digest.width);
    // The narrow Sources tab: the rail is the whole pane, and Digest stops at 360px.
    const wide = await measured(812);
    expect(wide.digest.width).toBeLessThanOrEqual(360);
    expect(wide.picker.width).toBe(wide.digest.width);
    expect(wide.digest.left).toBe(wide.zone.left);
  });
});
