// @vitest-environment node
/**
 * The Knowledge surfaces as fills, measured in a real layout engine (owner message 13, R-27;
 * design.md 9.11).
 *
 * `styles/knowledgeFill.test.ts` reads the authored rules at their source; this lays the real
 * components out in Chromium with those rules cut from `main.css` at their owners' markers, over the
 * token values the stylesheet itself declares (read through `scripts/lib/theme-tokens.mjs`, the way
 * `check-contrast.mjs` does), in light and dark. It pins what the source cannot:
 *  - the Sources drop target rests on the fill with a transparent 1px edge, hovers on the strong
 *    fill, takes the strong fill and an inset line while a file is held over it, and its plate rests
 *    one ground back, and none of it changes the box's size;
 *  - a graph filter is the filled control (the rest fill, the strong hover) and an engaged one is the
 *    strong fill under the selected wash, still 32px tall, with the label in the default ink;
 *  - the paste box is a filled card whose fields and Stage button rest on the page ground (a fill
 *    never sits on a fill);
 *  - on a recessed ground (a pane header, the graph canvas) a filled button rests on the strong fill
 *    and hovers one fill deeper, and one inside a filled box there still rests on the page ground;
 *  - forced colours draw the edge the fill dropped (the transparent border is kept).
 * Set `KNOWLEDGE_FILL_SHOTS` to a directory to also save the screenshots the review reads.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import * as themeTokens from '../../../scripts/lib/theme-tokens.mjs';
import type { Graph, GraphNode } from '../../api/types.gen';
import { Sparkles } from '../icons/app-icons';
import { Button } from '../ui/button';
import { EmptyState } from '../ui/empty-state';
import { Dropzone } from './IngestPanel/Dropzone';
import { PasteTextBox } from './IngestPanel/PasteTextBox';
import { GraphFacetStrip } from './graph/GraphFacetStrip';
import { EMPTY_FACETS } from './graph/graphFacets';
import { buildGraphModel } from './graph/graphModel';
import { KnowledgePaneHeader } from './KnowledgePaneHeader';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const SHOTS = process.env.KNOWLEDGE_FILL_SHOTS;

type Scope = { decls: Record<string, string>; theme: Record<string, string> };
const tokens = themeTokens as unknown as {
  buildScopes(css: string): Record<string, Scope>;
  resolveHex(name: string, scope: Scope): string | null;
};
const scopes = tokens.buildScopes(MAIN);
const MODES = { light: 'parchment:light', dark: 'parchment:dark' } as const;
type Mode = keyof typeof MODES;

/** Every neutral the surfaces read, as the stylesheet declares it in that mode. */
function tokenCss(mode: Mode): string {
  const scope = scopes[MODES[mode]];
  const names = [
    '--background-default',
    '--background-fill',
    '--background-fill-strong',
    '--background-strong',
    '--background-muted',
    '--background-focus',
    '--sidebar',
    '--text-default',
    '--text-muted',
    '--border-subtle',
    '--border-strong',
    '--border-focus',
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
  --overlay-selected: color-mix(in oklab, var(--text-default) 14%, transparent);
  --overlay-selected-hover: color-mix(in oklab, var(--text-default) 19%, transparent);
  --radius-element: 8px; --radius-container: 12px; --radius-inner: 4px;
  --chrome-height: 44px;
  --dur-fast-min: 0s; --ease-out: ease;
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

/** The block that opens at `begin` (a selector or an at-rule), through its matching brace. */
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

/** The Knowledge rules under test, and the shared fill primitives they stand on. */
function ruleCss(): string {
  return [
    // The shared control ground and the filled button and field recipe (WS-PRIMITIVES), as built.
    block('.br-fill-surface > *,'),
    block('@layer components {\n  .br-field,\n  .br-fill-rest {'),
    cut('/* @ws WS-KNOWLEDGE begin: dropzone */', '/* @ws WS-KNOWLEDGE end: dropzone */').replace(
      /@media \(prefers-reduced-motion: reduce\)[\s\S]*?\n\}\n/,
      ''
    ),
    cut('/* @ws WS-KNOWLEDGE begin: filter facet */', '/* @ws WS-KNOWLEDGE end: filter facet */'),
    cut(
      '/* @ws WS-KNOWLEDGE begin: recessed ground */',
      '/* @ws WS-KNOWLEDGE end: recessed ground */'
    ),
  ].join('\n');
}

/** The few utilities the markup needs to lay out; colours come from the rules above. */
const UTILITIES = `@layer base {
* { box-sizing: border-box; }
body { margin: 0; padding: 16px; font: 14px/20px var(--font-body); color: var(--text-default); }
button, input, textarea { font: inherit; color: inherit; }
button { cursor: pointer; border: 0 solid; background-color: transparent; }
svg { width: 16px; height: 16px; flex: none; }
}
@layer utilities {
.flex { display: flex; } .inline-flex { display: inline-flex; } .flex-col { flex-direction: column; }
.items-center { align-items: center; } .justify-center { justify-content: center; }
.flex-none, .flex-shrink-0, .shrink-0 { flex: none; } .relative { position: relative; }
.gap-1 { gap: 4px; } .gap-2 { gap: 8px; } .p-3 { padding: 12px; } .px-4 { padding-inline: 16px; }
.py-4 { padding-block: 16px; } .px-2\\.5 { padding-inline: 10px; } .px-3 { padding-inline: 12px; }
.px-2 { padding-inline: 8px; } .pl-7 { padding-left: 28px; }
.h-8 { height: 32px; } .w-8 { width: 32px; } .h-control-md { height: 32px; } .h-control-sm { height: 28px; }
.h-chrome { height: var(--chrome-height); } .w-\\[200px\\] { width: 200px; }
.rounded-element { border-radius: var(--radius-element); } .rounded-container { border-radius: var(--radius-container); }
.text-center { text-align: center; } .text-label { font-weight: 500; }
.text-supporting, .text-chip { font-size: 12px; line-height: 16px; }
.hidden { display: none; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
.absolute { position: absolute; } .pointer-events-none { pointer-events: none; }
.border { border: 1px solid; } .border-transparent { border-color: transparent; }
.border-b { border-bottom: 1px solid; } .border-border-subtle { border-color: var(--border-subtle); }
.bg-background-fill { background-color: var(--background-fill); }
.bg-sidebar { background-color: var(--sidebar); }
}
.biorouter-focus-surface:focus-visible { background-color: var(--background-focus); }
/* The facet strip shows its core step, as at the 860px pane. */
.br-facet-core, .br-facet-extra, .br-facet-more, .br-facet-all, .br-facet-legend { display: inline-flex; }
.br-facet-all, .br-facet-more, .br-facet-legend { display: none; }`;

function node(id: string, type?: string): GraphNode {
  return { id, label: id, kind: 'entity', path: `${id}.md`, node_type: type } as GraphNode;
}
const GRAPH: Graph = {
  nodes: [node('a', 'Drug'), node('b', 'Drug'), node('c', 'Disease')],
  edges: [],
};

function stripMarkup(engaged: boolean): string {
  return renderToStaticMarkup(
    <GraphFacetStrip
      model={buildGraphModel(GRAPH)}
      mode="light"
      facets={engaged ? { ...EMPTY_FACETS, types: new Set(['Drug']) } : EMPTY_FACETS}
      onChange={() => undefined}
      passing={2}
      total={3}
      active={engaged}
    />
  );
}

function documentFor(mode: Mode): string {
  const rail = renderToStaticMarkup(
    <div id="rail" style={{ width: 288, background: 'var(--background-default)', padding: 16 }}>
      <Dropzone
        onFiles={() => undefined}
        onPathPickRequested={() => undefined}
        onPasteText={() => undefined}
      />
      <div style={{ height: 12 }} />
      <PasteTextBox onStage={() => undefined} onCancel={() => undefined} />
    </div>
  );
  const header = renderToStaticMarkup(
    <KnowledgePaneHeader
      title={<h2 className="text-label">Check for problems</h2>}
      actions={
        <Button id="check" variant="secondary" size="sm">
          Check again
        </Button>
      }
      closeLabel="Close check"
    />
  );
  const canvas = renderToStaticMarkup(
    <div
      id="canvas"
      className="br-knowledge-recessed"
      style={{ background: 'var(--background-muted)', padding: 24, width: 360 }}
    >
      <Button id="retry" variant="secondary">
        Try again
      </Button>
      <EmptyState icon={Sparkles} title="Nothing digested yet" description="Stage a source." />
      <div
        id="card"
        className="br-fill-surface rounded-container border border-transparent bg-background-fill p-3"
        style={{ marginTop: 12 }}
      >
        <Button id="inner" variant="secondary">
          Inside a card
        </Button>
      </div>
    </div>
  );
  return `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${tokenCss(mode)}\n${UTILITIES}\n${ruleCss()}</style></head><body>
    ${rail}
    <div id="strip-rest" style="margin-top:12px;background:var(--background-default)">${stripMarkup(false)}</div>
    <div id="strip-on" style="margin-top:12px;background:var(--background-default)">${stripMarkup(true)}</div>
    <div id="header" style="margin-top:12px;width:340px">${header}</div>
    <div style="margin-top:12px">${canvas}</div>
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

/** `#rrggbb` as the `rgb(r, g, b)` Chromium reports. */
const rgb = (hex: string) => {
  const n = parseInt(hex.slice(1), 16);
  return `rgb(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255})`;
};
const isClear = (colour: string) =>
  colour === 'transparent' || /^rgba\(.*,\s*0\)$/.test(colour.replace(/\s+/g, ' '));

type Look = {
  bg: string;
  image: string;
  edge: string;
  edgeWidth: string;
  shadow: string;
  w: number;
  h: number;
};
const look = (page: Page, selector: string): Promise<Look> =>
  page.evaluate((sel) => {
    const el = document.querySelector(sel)!;
    const cs = getComputedStyle(el);
    const box = el.getBoundingClientRect();
    return {
      bg: cs.backgroundColor,
      image: cs.backgroundImage,
      edge: cs.borderTopColor,
      edgeWidth: cs.borderTopWidth,
      shadow: cs.boxShadow,
      w: Math.round(box.width * 10) / 10,
      h: Math.round(box.height * 10) / 10,
    };
  }, selector);

describe.each(Object.keys(MODES) as Mode[])('the Knowledge surfaces are fills (%s)', (mode) => {
  let browser: Browser | null = null;
  let page: Page | null = null;
  const scope = scopes[MODES[mode]];
  const hex = (name: string) => tokens.resolveHex(name, scope)!;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Knowledge fill tests.');
      return;
    }
    page = await browser.newPage({ viewport: { width: 640, height: 1100 } });
    await page.setContent(documentFor(mode));
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  it('rests the drop target on the fill with a transparent edge, and hovers on the strong fill', async (ctx) => {
    if (!page) return ctx.skip();
    const zone = '[data-testid="knowledge-ingest-dropzone"]';
    const rest = await look(page, zone);
    expect(rest.bg).toBe(rgb(hex('--background-fill')));
    expect(rest.edgeWidth).toBe('1px');
    expect(isClear(rest.edge), 'no resting outline').toBe(true);

    await page.hover(zone);
    const hover = await look(page, zone);
    expect(hover.bg).toBe(rgb(hex('--background-fill-strong')));
    // Nothing moves between rest and hover.
    expect([hover.w, hover.h]).toEqual([rest.w, rest.h]);
    await page.mouse.move(0, 0);

    // A file held over it: the strong fill and an inset line, drawn inside the same box.
    await page.evaluate(
      (sel) => document.querySelector(sel)!.setAttribute('data-dragging', 'true'),
      zone
    );
    const drag = await look(page, zone);
    expect(drag.bg).toBe(rgb(hex('--background-fill-strong')));
    expect(drag.shadow).toContain(rgb(hex('--border-strong')));
    expect(drag.shadow).toContain('inset');
    expect([drag.w, drag.h]).toEqual([rest.w, rest.h]);
    await page.evaluate(
      (sel) => document.querySelector(sel)!.removeAttribute('data-dragging'),
      zone
    );
  });

  it('draws the glyph plate on the page ground, with no edge', async (ctx) => {
    if (!page) return ctx.skip();
    const plate = await look(page, '.br-knowledge-drop-plate');
    expect(plate.bg).toBe(rgb(hex('--background-default')));
    expect(plate.edgeWidth).toBe('0px');
  });

  it('keeps the paste box a filled card whose controls rest on the page ground', async (ctx) => {
    if (!page) return ctx.skip();
    const card = await look(page, '#rail .br-fill-surface');
    expect(card.bg).toBe(rgb(hex('--background-fill')));
    expect(isClear(card.edge)).toBe(true);
    expect(card.edgeWidth).toBe('1px');
    // A fill never sits on a fill: the title field and the Stage button are one ground back.
    const field = await look(page, 'input[aria-label="Title for the pasted source"]');
    expect(field.bg).toBe(rgb(hex('--background-default')));
    expect(isClear(field.edge)).toBe(true);
    const stage = await look(page, '#rail .br-fill-surface button.br-fill-rest');
    expect(stage.bg).toBe(rgb(hex('--background-default')));
  });

  it('rests a graph filter on the fill, hovers it on the strong fill, and keeps its height', async (ctx) => {
    if (!page) return ctx.skip();
    const facet = '#strip-rest [data-testid="knowledge-graph-facet-type"]';
    const rest = await look(page, facet);
    expect(rest.bg).toBe(rgb(hex('--background-fill')));
    expect(rest.edgeWidth).toBe('1px');
    expect(isClear(rest.edge), 'no resting outline').toBe(true);
    expect(rest.h).toBe(32);
    await page.hover(facet);
    expect((await look(page, facet)).bg).toBe(rgb(hex('--background-fill-strong')));
    await page.mouse.move(0, 0);
  });

  it('draws an engaged filter as the strong fill under the selected wash, in the default ink', async (ctx) => {
    if (!page) return ctx.skip();
    const on = '#strip-on [data-testid="knowledge-graph-facet-type"]';
    const rest = await look(page, '#strip-rest [data-testid="knowledge-graph-facet-type"]');
    const engaged = await look(page, on);
    expect(engaged.bg).toBe(rgb(hex('--background-fill-strong')));
    expect(engaged.image).toContain('linear-gradient');
    expect(engaged.h).toBe(32);
    // Neutral: no accent anywhere, so the label is the ordinary ink.
    const ink = await page.evaluate(
      (sel) => getComputedStyle(document.querySelector(sel)!).color,
      on
    );
    expect(ink).toBe(rgb(hex('--text-default')));
    expect(rest.image).toBe('none');
  });

  it('rests a filled button on the strong fill on a recessed ground, and hovers one fill deeper', async (ctx) => {
    if (!page) return ctx.skip();
    for (const sel of ['#check', '#retry']) {
      const rest = await look(page, sel);
      expect(rest.bg, `${sel} rests on the strong fill`).toBe(rgb(hex('--background-fill-strong')));
      await page.hover(sel);
      expect((await look(page, sel)).bg, `${sel} hovers one fill deeper`).toBe(
        rgb(hex('--background-strong'))
      );
      await page.mouse.move(0, 0);
    }
    // A button inside a filled card on the same ground still rests one ground back.
    const inner = await look(page, '#inner');
    expect(inner.bg).toBe(rgb(hex('--background-default')));
  });

  it('draws an empty state’s mark tile on the strong fill there, where the rest fill is invisible', async (ctx) => {
    if (!page) return ctx.skip();
    const tile = await look(page, '#canvas section > div');
    expect(tile.bg).toBe(rgb(hex('--background-fill-strong')));
    // It is a step off the canvas it sits on, which the rest fill is not (1.00:1).
    const canvas = await look(page, '#canvas');
    expect(tile.bg).not.toBe(canvas.bg);
  });

  it('keeps the edge in forced colours, where the fill is repainted', async (ctx) => {
    if (!page) return ctx.skip();
    await page.emulateMedia({ forcedColors: 'active' });
    try {
      for (const sel of [
        '[data-testid="knowledge-ingest-dropzone"]',
        '#strip-rest [data-testid="knowledge-graph-facet-type"]',
        '#rail .br-fill-surface',
      ]) {
        const forced = await look(page, sel);
        expect(isClear(forced.edge), `${sel}: the transparent border is drawn`).toBe(false);
        expect(forced.edgeWidth).toBe('1px');
      }
    } finally {
      await page.emulateMedia({ forcedColors: 'none' });
    }
  });

  it('saves the screenshots the review reads', async (ctx) => {
    if (!page || !SHOTS) return ctx.skip();
    mkdirSync(SHOTS, { recursive: true });
    await page.screenshot({ path: join(SHOTS, `fills-${mode}.png`), fullPage: true });
  });
});
