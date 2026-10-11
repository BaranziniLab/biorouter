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
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
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

/** Set to a directory to also save the held-file screenshots a review reads. */
const SHOTS = process.env.SOURCES_RAIL_SHOTS;

/** Each case opens several pages; a loaded machine (four workers, other Chromiums) needs room. */
const CASE_TIMEOUT = 120_000;

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
  mode?: 'light' | 'dark';
  forcedColors?: boolean;
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
    { height = 760, staged = 0, paste = false, mode = 'light', forcedColors = false }: Options = {}
  ) {
    const tab = await browser!.newPage({
      viewport: { width: 1600, height: 1000 },
      reducedMotion: 'reduce',
    });
    if (forcedColors) await tab.emulateMedia({ forcedColors: 'active' });
    const errors: string[] = [];
    tab.on('pageerror', (error) => errors.push(error.message));
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}">` +
        `<head><style>${css}</style></head>` +
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

  it(
    'fills the rail’s free height with the dropzone, like a side panel',
    async (ctx) => {
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
        expect(m.zone.top - m.scroller.top, `${label}: dropzone at the top`).toBeCloseTo(
          m.padTop,
          0
        );
        expect(m.scrolls, `${label}: scrolls`).toBe(false);
        // The plate, the two lines and Paste text sit in the middle of the box, as one group.
        const mid = (m.zone.top + m.zone.bottom) / 2;
        expect(Math.abs((m.content.top + m.content.bottom) / 2 - mid), label).toBeLessThan(2);
      }
    },
    CASE_TIMEOUT
  );

  it(
    'yields its room to the paste box and the staged list, and stays a generous target',
    async (ctx) => {
      if (!browser) return ctx.skip();
      const empty = await measured(1152);

      // The paste box opens under it: the box gives back exactly the box and one gap.
      const paste = await measured(1152, { paste: true });
      expectFramed(paste, 'paste');
      expectOneGrower(paste, 'paste');
      expect(given(paste) - given(empty), 'the paste box and its gap').toBeGreaterThan(200);
      expect(
        Math.abs(paste.zone.height - (empty.zone.height - yielded(empty, paste)))
      ).toBeLessThan(2);
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
    },
    CASE_TIMEOUT
  );

  it(
    'caps the model field and Digest at one column, left with the dropzone',
    async (ctx) => {
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
    },
    CASE_TIMEOUT
  );
  /** The colour Chromium painted at one viewport pixel, read back from a screenshot. */
  async function pixel(tab: Page, x: number, y: number): Promise<number[]> {
    const png = await tab.screenshot({ clip: { x, y, width: 1, height: 1 } });
    return tab.evaluate(async (b64) => {
      const blob = await (await fetch(`data:image/png;base64,${b64}`)).blob();
      const canvas = new OffscreenCanvas(1, 1);
      const context = canvas.getContext('2d')!;
      context.drawImage(await createImageBitmap(blob), 0, 0);
      return [...context.getImageData(0, 0, 1, 1).data.slice(0, 3)];
    }, png.toString('base64'));
  }

  /** What a token paints in this document, as [r, g, b]. */
  const paint = (tab: Page, value: string) =>
    tab.evaluate((v) => {
      const probe = document.createElement('div');
      probe.style.backgroundColor = v;
      document.body.append(probe);
      const out = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return (out.match(/\d+/g) ?? []).slice(0, 3).map(Number);
    }, value);

  const near = (a: number[], b: number[]) => a.every((v, i) => Math.abs(v - b[i]) <= 2);

  /** WCAG's contrast ratio between two sRGB colours. */
  function contrast(a: number[], b: number[]): number {
    const luminance = (c: number[]) => {
      const [r, g, bl] = c.map((v) => {
        const s = v / 255;
        return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
    };
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  }

  /** A file held over (or taken off) the last staged row, as the browser reports a drag. */
  const hold = (tab: Page, type: 'dragenter' | 'dragleave') =>
    tab.evaluate((t) => {
      const rows = document.querySelectorAll('[data-testid="knowledge-staged-item"]');
      const files = new DataTransfer();
      files.items.add(new File(['notes'], 'held.md', { type: 'text/markdown' }));
      rows[rows.length - 1].dispatchEvent(
        new DragEvent(t, { bubbles: true, cancelable: true, dataTransfer: files })
      );
    }, type);

  /** The rail's column as a PNG, and the same rail scrolled back to its box. */
  async function shoot(tab: Page, name: string) {
    mkdirSync(SHOTS!, { recursive: true });
    const clip = await tab.evaluate(() => {
      const r = document.querySelector('#sources')!.getBoundingClientRect();
      return { x: r.left, y: r.top, width: r.width, height: r.height };
    });
    await tab.screenshot({ path: join(SHOTS!, `${name}.png`), clip });
    const scroller =
      'document.querySelector(\'[data-testid="knowledge-ingest-dropzone"]\').parentElement';
    await tab.evaluate(`${scroller}.scrollTop = 0`);
    await tab.screenshot({ path: join(SHOTS!, `${name.replace('scrolled', 'top')}.png`), clip });
    await tab.evaluate(`${scroller}.scrollTop = ${scroller}.scrollHeight`);
  }

  /** The rail with a long list, scrolled to its end, so the box is out of view above it. */
  async function scrolledAway(options: Options) {
    const tab = await open(1152, { staged: 30, ...options });
    const geometry = await tab.evaluate(() => {
      const zone = document.querySelector('[data-testid="knowledge-ingest-dropzone"]')!;
      const scroller = zone.parentElement!;
      scroller.scrollTop = scroller.scrollHeight;
      const rail = document.querySelector('[data-testid="knowledge-ingest-rail"]')!;
      const r = rail.getBoundingClientRect();
      const s = scroller.getBoundingClientRect();
      const rows = document.querySelectorAll('[data-testid="knowledge-staged-item"]');
      const row = rows[rows.length - 1].getBoundingClientRect();
      return {
        zoneBottom: zone.getBoundingClientRect().bottom,
        scrollerTop: s.top,
        // One pixel on each of the rail's four edges: the left and right in the scroller's
        // middle, the top over the scrolled list, the bottom over the footer's own ground.
        edges: {
          left: [Math.round(r.left), Math.round((s.top + s.bottom) / 2)],
          right: [Math.round(r.right) - 1, Math.round((s.top + s.bottom) / 2)],
          top: [Math.round((r.left + r.right) / 2), Math.round(r.top)],
          bottom: [Math.round((r.left + r.right) / 2), Math.round(r.bottom) - 1],
        } as Record<string, [number, number]>,
        // One pixel in, on the scroller's padding: the edge is one pixel and no more.
        inside: [Math.round(r.left) + 2, Math.round((s.top + s.bottom) / 2)] as [number, number],
        row: [Math.round(row.left + 40), Math.round((row.top + row.bottom) / 2)] as [
          number,
          number,
        ],
      };
    });
    return { tab, ...geometry };
  }

  it(
    'answers a held file on the rail itself, with the box scrolled out of view',
    async (ctx) => {
      if (!browser) return ctx.skip();
      for (const mode of ['light', 'dark'] as const) {
        const { tab, zoneBottom, scrollerTop, edges, inside, row } = await scrolledAway({ mode });
        try {
          // The case the finding names: the box is above the scroller's top, out of view.
          expect(zoneBottom, `${mode}: the box is out of view`).toBeLessThanOrEqual(scrollerTop);
          // The composer card's drag-over edge, and 3:1 on the ground, as a state should be.
          const edge = await paint(tab, 'var(--border-accent)');
          const ground = await paint(tab, 'var(--background-default)');
          expect(contrast(edge, ground), `${mode}: the edge on the ground`).toBeGreaterThanOrEqual(
            3
          );
          // At rest the rail draws no edge of its own on the left, top and bottom.
          for (const side of ['left', 'top', 'bottom']) {
            const [x, y] = edges[side];
            expect(near(await pixel(tab, x, y), ground), `${mode}: ${side} at rest`).toBe(true);
          }

          await hold(tab, 'dragenter');
          await tab.waitForSelector('[data-testid="knowledge-ingest-rail"][data-dragging="true"]');
          if (SHOTS) await shoot(tab, `rail-held-scrolled-${mode}`);
          for (const side of Object.keys(edges)) {
            const [x, y] = edges[side];
            const got = await pixel(tab, x, y);
            expect(near(got, edge), `${mode}: ${side} edge while held, got ${got}`).toBe(true);
          }
          expect(near(await pixel(tab, ...inside), ground), `${mode}: one pixel`).toBe(true);
          // The edge takes no pointer: a file let go on a row still lands on the row.
          const hit = await tab.evaluate(
            ([x, y]) =>
              document.elementFromPoint(x, y)?.closest('[data-testid="knowledge-staged-item"]') !==
              null,
            row
          );
          expect(hit, `${mode}: the row is still the drop's target`).toBe(true);

          await hold(tab, 'dragleave');
          await tab.waitForSelector('[data-testid="knowledge-ingest-rail"]:not([data-dragging])');
          const [x, y] = edges.left;
          expect(near(await pixel(tab, x, y), ground), `${mode}: gone on leaving`).toBe(true);
        } finally {
          await tab.close();
        }
      }
    },
    CASE_TIMEOUT
  );

  it(
    'keeps the held-file edge in forced colours',
    async (ctx) => {
      if (!browser) return ctx.skip();
      const { tab, edges } = await scrolledAway({ forcedColors: true });
      try {
        const [x, y] = edges.left;
        const rest = await pixel(tab, x, y);
        await hold(tab, 'dragenter');
        await tab.waitForSelector('[data-testid="knowledge-ingest-rail"][data-dragging="true"]');
        const held = await pixel(tab, x, y);
        expect(near(held, rest), `forced colours: ${rest} at rest, ${held} held`).toBe(false);
      } finally {
        await tab.close();
      }
    },
    CASE_TIMEOUT
  );
});
