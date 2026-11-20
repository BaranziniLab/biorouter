// @vitest-environment node
/**
 * The Knowledge graph canvas's shape, measured in a real layout engine (owner, 2026-11-19).
 *
 * The owner saw two things on the Knowledge page: a "weird line" along the top of the graph, with a
 * stray mark at its top-left corner, and a page of rounded panels with one square one, the graph
 * canvas, running into the page edges. This renders the real `KnowledgeGraphPanel` (the
 * force-graph library is replaced by an empty box, because a canvas needs a GPU context and none of
 * this is about what it draws) to markup and lays it out in Chromium inside the pane's container,
 * with the Knowledge CSS cut from `main.css` at its owners' markers, over the token values the
 * stylesheet declares (read through `scripts/lib/theme-tokens.mjs`). It pins:
 *  - the canvas is an inset panel on the container radius, 16px in from the column's sides and foot
 *    like the Sources dropzone, starting at the filter bar's foot, and 16px from the top as well
 *    when there is no filter bar;
 *  - nothing draws a line between the filter bar and the canvas: the strip has no bottom edge, and
 *    the pixels across that seam are the page ground and then the canvas ground;
 *  - the canvas's zero-size probes paint nothing (the border probe was a 2x2 dot at the corner), and
 *    the border probe still reads the token's colour;
 *  - the read-only preview banner is an inset rounded notice, not a square full-bleed strip;
 *  - at the widest step the legend rail sits beside the canvas with no divider of its own.
 * jsdom lays nothing out and evaluates no container query, which is why this lives here.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import * as themeTokens from '../../../scripts/lib/theme-tokens.mjs';
import type { Graph, GraphNode } from '../../api/types.gen';
import { KnowledgeGraphPanel } from './graph/KnowledgeGraphPanel';

// The library draws into a <canvas> after mount; markup rendering never mounts, and the box is all
// the layout needs.
vi.mock('react-force-graph-2d', () => ({
  default: () => <div data-testid="force-graph-stub" />,
}));

const here = dirname(fileURLToPath(import.meta.url));
const MAIN = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

type Scope = { decls: Record<string, string>; theme: Record<string, string> };
const tokens = themeTokens as unknown as {
  buildScopes(css: string): Record<string, Scope>;
  resolveHex(name: string, scope: Scope): string | null;
};
const scopes = tokens.buildScopes(MAIN);
const MODES = { light: 'parchment:light', dark: 'parchment:dark' } as const;
type Mode = keyof typeof MODES;

function tokenCss(mode: Mode): string {
  const scope = scopes[MODES[mode]];
  const names = [
    '--background-default',
    '--background-fill',
    '--background-fill-strong',
    '--background-strong',
    '--background-muted',
    '--text-default',
    '--text-muted',
    '--border-subtle',
  ];
  const decls = names
    .map((name) => {
      const value = tokens.resolveHex(name, scope);
      if (!value) throw new Error(`${name} does not resolve to a hex in ${MODES[mode]}`);
      return `${name}: ${value};`;
    })
    .join('\n  ');
  return `:root {
  ${decls}
  --radius-element: 8px; --radius-container: 12px; --radius-inner: 4px;
  /* A wash is a mix, not a hex; the notice's shape is measured, never its colour. */
  --wash-warning: rgb(200 150 50 / 0.22); --text-warning: #8a5a00;
  --chrome-height: 44px;
  --knowledge-rail-sources: 300px; --knowledge-rail-detail: 340px;
  --font-body: Arial, Helvetica, sans-serif;
}`;
}

/** A rule block of `main.css`, cut between two markers (inclusive of everything between). */
function cut(begin: string, end: string): string {
  const from = MAIN.indexOf(begin);
  const to = MAIN.indexOf(end, from);
  if (from < 0 || to < from) throw new Error(`markers not found in main.css: ${begin}`);
  return MAIN.slice(from, to + end.length);
}

/** The block that opens at `begin`, through its matching brace. */
function block(begin: string): string {
  const from = MAIN.indexOf(begin);
  if (from < 0) throw new Error(`block not found in main.css: ${begin}`);
  let depth = 0;
  for (let at = MAIN.indexOf('{', from); at < MAIN.length; at++) {
    if (MAIN[at] === '{') depth++;
    if (MAIN[at] === '}' && --depth === 0) return MAIN.slice(from, at + 1);
  }
  throw new Error(`unbalanced block in main.css: ${begin}`);
}

/** The pane container, its ladder and every Knowledge rule up to the filter facet, as built. */
function ruleCss(): string {
  return [
    cut('.br-knowledge-pane {', '/* @ws WS-KNOWLEDGE begin: filter facet */'),
    cut('/* @ws WS-KNOWLEDGE begin: filter facet */', '/* @ws WS-KNOWLEDGE end: filter facet */'),
    block('.br-ground-recessed {'),
  ].join('\n');
}

/**
 * The utilities the markup uses, as Tailwind emits them: in the utilities LAYER, over a preflight
 * that gives every element `border: 0 solid`, so a `border` utility is what gives a box an edge.
 */
const UTILITIES = `@layer base {
*, ::before, ::after { box-sizing: border-box; margin: 0; padding: 0; border: 0 solid; }
body { font: 14px/20px var(--font-body); color: var(--text-default); background: var(--background-default); }
button, input { font: inherit; color: inherit; background: transparent; }
svg { width: 16px; height: 16px; flex: none; }
}
@layer utilities {
.flex { display: flex; } .inline-flex { display: inline-flex; } .flex-col { flex-direction: column; }
.items-center { align-items: center; } .items-start { align-items: flex-start; }
.justify-center { justify-content: center; } .justify-between { justify-content: space-between; }
.flex-1 { flex: 1 1 0%; } .flex-none, .shrink-0 { flex: none; } .min-h-0 { min-height: 0; } .min-w-0 { min-width: 0; }
.relative { position: relative; } .absolute { position: absolute; } .inset-0 { inset: 0; }
.h-full { height: 100%; } .w-full { width: 100%; } .h-0 { height: 0; } .w-0 { width: 0; }
.overflow-hidden { overflow: hidden; } .overflow-y-auto { overflow-y: auto; }
.pointer-events-none { pointer-events: none; }
.gap-1 { gap: 4px; } .gap-2 { gap: 8px; } .px-4 { padding-inline: 16px; } .px-3 { padding-inline: 12px; }
.px-2 { padding-inline: 8px; } .py-2 { padding-block: 8px; } .py-2\\.5 { padding-block: 10px; } .pl-7 { padding-left: 28px; }
.mx-4 { margin-inline: 16px; } .mt-4 { margin-top: 16px; } .ml-auto { margin-left: auto; }
.h-chrome { height: var(--chrome-height); } .h-control-md { height: 32px; } .w-\\[200px\\] { width: 200px; }
.rounded-element { border-radius: var(--radius-element); } .rounded-container { border-radius: var(--radius-container); }
.rounded-inner { border-radius: var(--radius-inner); }
.border { border-width: 1px; border-style: solid; } .border-transparent { border-color: transparent; }
.border-b { border-bottom-width: 1px; border-bottom-style: solid; }
.border-l { border-left-width: 1px; border-left-style: solid; }
.border-border-subtle { border-color: var(--border-subtle); }
.bg-background-default { background-color: var(--background-default); }
.bg-background-fill { background-color: var(--background-fill); }
.bg-background-muted { background-color: var(--background-muted); }
.bg-wash-warning { background-color: var(--wash-warning); }
.text-text-warning { color: var(--text-warning); } .text-text-muted { color: var(--text-muted); }
.text-supporting { font-size: 12px; line-height: 16px; }
.opacity-0 { opacity: 0; } .opacity-100 { opacity: 1; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
.hidden { display: none; }
}`;

function node(id: string, type: string): GraphNode {
  return {
    id,
    label: id,
    kind: 'entity',
    path: `knowledge/${id}.md`,
    node_type: type,
  } as GraphNode;
}
const GRAPH: Graph = {
  nodes: [node('hrv', 'Concept'), node('vagal', 'Concept'), node('chen', 'Source')],
  edges: [
    { from: 'hrv', to: 'vagal' },
    { from: 'chen', to: 'hrv' },
  ],
};

function panelMarkup(graph: Graph, previewSha: string | null = null): string {
  return renderToStaticMarkup(
    <KnowledgeGraphPanel
      kbId="kb-1"
      graph={graph}
      loading={false}
      error={null}
      onRefresh={() => undefined}
      previewSha={previewSha}
      onClearPreview={() => undefined}
    />
  );
}

/** The panel as `KnowledgeView` mounts it: the graph column of the pane's two-column body. */
function documentFor(mode: Mode, paneWidth: number, panel: string): string {
  return `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>@layer base, components, utilities;
${tokenCss(mode)}
${UTILITIES}
${ruleCss()}</style></head><body>
    <div class="br-knowledge-pane" id="pane" style="width:${paneWidth}px;height:720px;display:flex;flex-direction:column">
      <div class="br-knowledge-body" style="flex:1 1 auto;height:720px">
        <div class="br-knowledge-sources" style="display:flex"></div>
        <div id="column" class="br-knowledge-graph flex min-h-0 min-w-0">${panel}</div>
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

type Box = { left: number; right: number; top: number; bottom: number };
const box = (page: Page, selector: string): Promise<Box> =>
  page.evaluate((sel) => {
    const r = document.querySelector(sel)!.getBoundingClientRect();
    return { left: r.left, right: r.right, top: r.top, bottom: r.bottom };
  }, selector);
const style = (page: Page, selector: string, property: string): Promise<string> =>
  page.evaluate(
    ([sel, prop]) => getComputedStyle(document.querySelector(sel)!).getPropertyValue(prop),
    [selector, property] as const
  );

type Rgb = [number, number, number];
/** The colour the page paints at one CSS pixel. */
async function pixel(page: Page, x: number, y: number): Promise<Rgb> {
  const shot = await page.screenshot({
    clip: { x: Math.round(x), y: Math.round(y), width: 1, height: 1 },
  });
  return (await page.evaluate(async (b64) => {
    const img = new Image();
    img.src = 'data:image/png;base64,' + b64;
    await img.decode();
    const c = document.createElement('canvas');
    c.width = c.height = 1;
    const g = c.getContext('2d')!;
    g.drawImage(img, 0, 0);
    return Array.from(g.getImageData(0, 0, 1, 1).data.slice(0, 3));
  }, shot.toString('base64'))) as Rgb;
}
const hexRgb = (hex: string): Rgb => {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
};

describe.each(Object.keys(MODES) as Mode[])('the Knowledge graph canvas (%s)', (mode) => {
  let browser: Browser | null = null;
  const hex = (name: string) => tokens.resolveHex(name, scopes[MODES[mode]])!;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) console.warn('No Chromium found: skipping the Knowledge canvas tests.');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(paneWidth: number, panel: string): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width: 1700, height: 800 } });
    await page.setContent(documentFor(mode, paneWidth, panel));
    return page;
  }

  it('is an inset panel on the container radius, 16px in like the dropzone, from the filter bar down', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const width of [760, 1152]) {
      const page = await open(width, panelMarkup(GRAPH));
      try {
        const column = await box(page, '#column');
        const strip = await box(page, '[data-testid="knowledge-graph-facets"]');
        const canvas = await box(page, '[data-testid="knowledge-graph-ground"]');
        for (const corner of [
          'border-top-left-radius',
          'border-top-right-radius',
          'border-bottom-left-radius',
          'border-bottom-right-radius',
        ]) {
          expect(
            await style(page, '[data-testid="knowledge-graph-ground"]', corner),
            `${width}px ${corner}`
          ).toBe('12px');
        }
        expect(canvas.left - column.left, `${width}px left inset`).toBe(16);
        expect(column.right - canvas.right, `${width}px right inset`).toBe(16);
        expect(column.bottom - canvas.bottom, `${width}px foot inset`).toBe(16);
        // It starts at the filter bar's foot, so the bar's 32px controls sit centred between the
        // band and the canvas, and the search field's left edge is the canvas's.
        expect(canvas.top, `${width}px top`).toBe(strip.bottom);
        const search = await box(page, '[data-testid="knowledge-graph-search"]');
        expect(search.left, `${width}px search edge`).toBe(canvas.left);
        // The focusable surface inside takes the same corner, so its inset focus ring is not cut.
        expect(await style(page, '.br-knowledge-graph-surface', 'border-top-left-radius')).toBe(
          '12px'
        );
      } finally {
        await page.close();
      }
    }
  });

  it('keeps 16px on every side when there is no filter bar above it (an empty base)', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1152, panelMarkup({ nodes: [], edges: [] }));
    try {
      expect(await page.$('[data-testid="knowledge-graph-facets"]')).toBeNull();
      const column = await box(page, '#column');
      const canvas = await box(page, '[data-testid="knowledge-graph-ground"]');
      expect(canvas.top - column.top).toBe(16);
      expect(canvas.left - column.left).toBe(16);
      expect(column.right - canvas.right).toBe(16);
      expect(column.bottom - canvas.bottom).toBe(16);
    } finally {
      await page.close();
    }
  });

  it('draws no line between the filter bar and the canvas', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1152, panelMarkup(GRAPH));
    try {
      expect(
        await style(page, '[data-testid="knowledge-graph-facets"]', 'border-bottom-width')
      ).toBe('0px');
      const strip = await box(page, '[data-testid="knowledge-graph-facets"]');
      const canvas = await box(page, '[data-testid="knowledge-graph-ground"]');
      // Read across the seam well inside the canvas's straight top edge, clear of any control.
      const x = (canvas.left + canvas.right) / 2;
      expect(await pixel(page, x, strip.bottom - 1), 'the row above the seam').toEqual(
        hexRgb(hex('--background-default'))
      );
      expect(await pixel(page, x, canvas.top), 'the canvas’s first row').toEqual(
        hexRgb(hex('--background-muted'))
      );
    } finally {
      await page.close();
    }
  });

  it('paints nothing with its zero-size probes, and still reads the border colour', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1152, panelMarkup(GRAPH));
    try {
      const probes = await page.evaluate(() =>
        [
          ...document.querySelectorAll(
            '[data-testid="knowledge-graph-canvas"] > span[aria-hidden="true"]'
          ),
        ].map((probe) => {
          const r = probe.getBoundingClientRect();
          const cs = getComputedStyle(probe);
          return {
            w: r.width,
            h: r.height,
            edge: cs.borderTopWidth,
            colour: cs.borderTopColor,
            cls: probe.className,
          };
        })
      );
      expect(probes).toHaveLength(4);
      for (const probe of probes) {
        expect(probe.w, probe.cls).toBe(0);
        expect(probe.h, probe.cls).toBe(0);
        expect(probe.edge, probe.cls).toBe('0px');
      }
      // The border probe is the one that reads the token: its colour resolves at width 0.
      const border = probes.find((p) => /\bborder-border-subtle\b/.test(p.cls));
      const [r, g, b] = hexRgb(hex('--border-subtle'));
      expect(border?.colour).toBe(`rgb(${r}, ${g}, ${b})`);
    } finally {
      await page.close();
    }
  });

  it('shows the read-only preview as an inset rounded notice, not a full-bleed strip', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1152, panelMarkup(GRAPH, 'd904f7e1234567'));
    try {
      const column = await box(page, '#column');
      const note = await box(page, '[data-testid="knowledge-graph-preview-note"]');
      const canvas = await box(page, '[data-testid="knowledge-graph-ground"]');
      expect(
        await style(page, '[data-testid="knowledge-graph-preview-note"]', 'border-top-left-radius')
      ).toBe('8px');
      expect(note.left - column.left).toBe(16);
      expect(column.right - note.right).toBe(16);
      expect(note.top - column.top).toBe(16);
      // The notice shares the canvas's edges.
      expect(note.left).toBe(canvas.left);
      expect(note.right).toBe(canvas.right);
    } finally {
      await page.close();
    }
  });

  it('sets the legend rail beside the canvas at the widest step, with no divider of its own', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open(1500, panelMarkup(GRAPH));
    try {
      expect(await style(page, '[data-testid="knowledge-graph-legend"]', 'display')).toBe('flex');
      expect(await style(page, '[data-testid="knowledge-graph-legend"]', 'border-left-width')).toBe(
        '0px'
      );
      const canvas = await box(page, '[data-testid="knowledge-graph-ground"]');
      const rail = await box(page, '[data-testid="knowledge-graph-legend"]');
      expect(rail.left - canvas.right).toBe(16);
    } finally {
      await page.close();
    }
  });
});
