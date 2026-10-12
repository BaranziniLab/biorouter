// @vitest-environment node
/**
 * A list's markers in a tool's output well, painted by Chromium (merge-verify #391, screen 20).
 *
 * The output well re-voices markdown in the code role (13px mono, `transcript-row.css`), and the
 * markdown root clips its inline overflow (`overflow-x-hidden` in `MarkdownContent`). An outside
 * marker hangs left of its item, and in mono "1. " is three whole advances, wider than the prose
 * list's 1.5em gutter: the bug-report tool's five-step method read "l." for "1.", and a
 * twelve-item list lost the first digit of 10, 11 and 12 entirely.
 *
 * jsdom lays nothing out, and a `::marker` has no box any DOM API reports, so this reads PIXELS.
 * The real `ToolCallWithResponse` is bundled with esbuild (its component stylesheets included, as
 * the app imports them), `styles/main.css` is compiled by Tailwind over the bundle's classes and
 * run through the renderer's text-size pass, and each row is opened (and "Show more" pressed) as a
 * person would. For every item, the strip from the well's left edge to the item's text, one line
 * tall, is read twice: as painted, and with the markdown root's clip lifted (`overflow: visible`,
 * still a block formatting context, so nothing moves). The marker's leftmost ink in the unclipped
 * paint must sit on or right of the clip edge, and the two paints must hold the same ink: then no
 * part of any marker is cut, and all of it is inside the well.
 *
 * "The instrument" proves the cases can fail: the same page with the gutter rules taken back out
 * of `transcript-row.css` (the defect as found), and with only the one-digit gutter kept (a list
 * that reaches 10 is cut again). A mutation that matches nothing throws. Set
 * `LIST_MARKER_SHOTS_DIR` to save a screenshot of each page there.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../styles');
const requireFromDesktop = createRequire(resolve(here, '../../package.json'));
const SHOTS = process.env.LIST_MARKER_SHOTS_DIR;

/** `investigation_guidance` in `crates/biorouter/src/agents/bug_report/mod.rs`, at 1.92.1. */
const BUG_REPORT_METHOD =
  'Before you write the report, find out whether this is a defect:\n' +
  "1. Compare what the user expected with what happened, from this chat's own record.\n" +
  '2. Check whether the behaviour is documented and intended. The documentation is ' +
  'https://biorouter.ucsf.edu/docs (one large page; its sources are ' +
  '`docs/website/pages/<page>.html` in the repository, and contributor documentation ' +
  'is under `docs/`).\n' +
  '3. Read the code involved at the version the user runs: ' +
  'https://github.com/BaranziniLab/biorouter/tree/v1.92.1 (raw files: ' +
  'https://raw.githubusercontent.com/BaranziniLab/biorouter/v1.92.1/<path>). If that tag does ' +
  'not exist, use `main`; if `main` differs, the bug may already be fixed, so say so. ' +
  'If the working directory is a Biorouter checkout, read the code there instead. The ' +
  '`develop-biorouter` skill maps where code lives.\n' +
  '4. Use whatever web-fetch or shell tool you have. If you have none, say in ' +
  '`suspected_cause` that the source and the documentation were not checked.\n' +
  '5. If the documentation and the code show Biorouter behaved as designed, explain the ' +
  'intended use to the user instead of filing, and file only if they still want to (a ' +
  'misleading document or error message is itself worth reporting).\n';

const numbered = (from: number, count: number) =>
  Array.from({ length: count }, (_, i) => `${from + i}. step ${from + i}`).join('\n');

const CODE_BLOCK = '\n\n```\nmake test\n```\n\n';

/** One output per tool row, in page order. */
const OUTPUTS = {
  /** The #391 surface: one-digit numbers, long items that wrap. */
  bugReport: BUG_REPORT_METHOD,
  /** Two-digit numbers: the first digit of 10, 11 and 12 was cut whole. */
  twelve: numbered(1, 12),
  /**
   * CommonMark ends a list at an unindented code block and numbers the rest from where it was
   * (`<ol start="8">`). Three items from 8 reach 10.
   */
  continued: numbered(1, 7) + CODE_BLOCK + numbered(8, 3),
  /** A continued list whose numbers stay one digit. */
  continuedShort: numbered(1, 2) + CODE_BLOCK + numbered(3, 2),
  /** A list that starts at a two-digit number. */
  startsAtTwelve: numbered(12, 3),
  bullets: '- alpha\n- beta\n- gamma',
};
type CaseName = keyof typeof OUTPUTS;
const CASES = Object.keys(OUTPUTS) as CaseName[];

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import ToolCallWithResponse from './ToolCallWithResponse';

const outputs = (window as unknown as { __outputs: Array<[string, string]> }).__outputs;

function App() {
  return (
    <div style={{ width: 760, margin: '20px 0 0 100px' }}>
      {outputs.map(([name, text]) => (
        <div key={name} data-case={name} style={{ marginBottom: 16 }}>
          <ToolCallWithResponse
            isCancelledMessage={false}
            toolRequest={{
              type: 'toolRequest',
              id: name,
              toolCall: {
                status: 'success',
                value: { name: 'developer__shell', arguments: { command: 'true' } },
              },
            }}
            toolResponse={{
              type: 'toolResponse',
              id: name,
              toolResult: { status: 'success', value: { content: [{ type: 'text', text }] } },
            }}
            onOpenArtifact={() => {}}
          />
        </div>
      ))}
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
`;

/** The gutter rules as `transcript-row.css` writes them, to take back out. */
const ONE_DIGIT_GUTTER = `.br-transcript-row-well .biorouter-markdown.prose :where(ul, ol) {
  padding-inline-start: 3ch;
}`;
const WIDER_GUTTERS =
  /\.br-transcript-row-well\s+\.biorouter-markdown\.prose\s+ol:where\([\s\S]*?padding-inline-start: 5ch;\n\}/;

type Variant = 'fixed' | 'no-gutter' | 'one-digit-only';

/** The row stylesheet with one defect put back. Must match, or throw. */
function mutate(source: string, variant: Variant): string {
  if (variant === 'fixed') return source;
  if (!WIDER_GUTTERS.test(source)) throw new Error('mutation target missing: the 4ch/5ch rules');
  let out = source.replace(WIDER_GUTTERS, '');
  if (variant === 'no-gutter') {
    if (!out.includes(ONE_DIGIT_GUTTER)) throw new Error('mutation target missing: the 3ch rule');
    out = out.split(ONE_DIGIT_GUTTER).join('');
  }
  return out;
}

async function bundle(variant: Variant): Promise<{ script: string; css: string }> {
  const mutateRows: Plugin = {
    name: 'mutate-transcript-row',
    setup(b) {
      b.onLoad({ filter: /\/transcript-row\.css$/ }, (args) => ({
        contents: mutate(readFileSync(args.path, 'utf8'), variant),
        loader: 'css',
      }));
    },
  };
  const out = await build({
    stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
    bundle: true,
    write: false,
    // Only names the outputs (nothing is written): the component stylesheets come back as CSS.
    outdir: resolve(here, '.list-marker-bundle'),
    format: 'iife',
    jsx: 'automatic',
    platform: 'browser',
    define: { 'process.env.NODE_ENV': '"production"' },
    loader: {
      '.css': 'css',
      '.svg': 'dataurl',
      '.png': 'dataurl',
      '.woff': 'empty',
      '.woff2': 'empty',
      '.ttf': 'empty',
    },
    logLevel: 'silent',
    plugins: [mutateRows],
  });
  const script = out.outputFiles.find((f) => f.path.endsWith('.js'))!.text;
  const components = out.outputFiles.find((f) => f.path.endsWith('.css'))?.text ?? '';
  if (!components.includes('.br-transcript-row-well')) {
    throw new Error('the bundle carried no transcript-row.css: the wells would be unstyled');
  }
  const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
  const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
  const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
    base: STYLES,
    onDependency() {},
  });
  const tailwind = compiler.build(
    new Scanner({}).scanFiles([{ content: script, extension: 'js' }])
  );
  // main.css first and the component sheets after, the order the renderer loads them in.
  const sheet = await postcss([fontScale()]).process(tailwind + components, { from: undefined });
  return { script, css: sheet.css };
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

/** One list item's marker strip, in CSS px from the page's left edge. */
type Item = {
  list: number;
  index: number;
  /** The markdown root's left edge: its overflow clip. */
  clipEdge: number;
  wellLeft: number;
  /** The item's text edge (its content box). */
  textLeft: number;
  /** Leftmost ink in the strip, or null when the strip holds none. */
  inkLeft: number | null;
  inkPixels: number;
};

/** A case as read: its items, each list's gutter in the well's `ch`, and each list's text edge. */
type Reading = { items: Item[]; gutters: number[]; textEdges: number[] };
type Measured = Record<CaseName, { painted: Reading; unclipped: Reading }>;

/** Device pixels per CSS px: half-pixel reads, so a cut through a stem is not rounded away. */
const SCALE = 2;

/** Each list's gutter (in `ch` of its own font) and the text edge of its first item. */
const readLists = (tab: Page, name: CaseName) =>
  tab.evaluate((caseName) => {
    const md = document.querySelector(
      `[data-case='${caseName}'] .br-transcript-row-well .biorouter-markdown`
    )!;
    const lists = Array.from(md.querySelectorAll(':scope > ol, :scope > ul'));
    return {
      gutters: lists.map((list) => {
        const probe = document.createElement('span');
        probe.style.cssText = 'display:inline-block;width:1ch;position:absolute';
        list.appendChild(probe);
        const ch = probe.getBoundingClientRect().width;
        probe.remove();
        // Layout snaps the probe to 1/64px, so tenths are what the gutter is counted in.
        return Math.round((parseFloat(getComputedStyle(list).paddingInlineStart) / ch) * 10) / 10;
      }),
      textEdges: lists.map((list) => {
        const li = list.querySelector(':scope > li')!;
        return li.getBoundingClientRect().left + parseFloat(getComputedStyle(li).paddingLeft);
      }),
    };
  }, name);

/** Read every item's marker strip in one case's well, as painted now. */
async function readStrips(tab: Page, name: CaseName): Promise<Item[]> {
  const geometry = await tab.evaluate((caseName) => {
    const root = document.querySelector(`[data-case='${caseName}']`)!;
    const well = root.querySelector('.br-transcript-row-well')!.getBoundingClientRect();
    const md = root.querySelector('.br-transcript-row-well .biorouter-markdown')!;
    const items: Array<{ list: number; index: number; top: number; textLeft: number }> = [];
    md.querySelectorAll(':scope > ol, :scope > ul').forEach((list, listIndex) => {
      list.querySelectorAll(':scope > li').forEach((li, index) => {
        const box = li.getBoundingClientRect();
        const pad = parseFloat(getComputedStyle(li).paddingLeft);
        items.push({ list: listIndex, index, top: box.top, textLeft: box.left + pad });
      });
    });
    return {
      well: { x: well.left, y: well.top, width: well.width, height: well.height },
      clipEdge: md.getBoundingClientRect().left,
      lineHeight: parseFloat(getComputedStyle(md).lineHeight),
      items,
    };
  }, name);
  const shot = await tab.screenshot({ clip: geometry.well });
  return tab.evaluate(
    async ({ b64, g, scale }) => {
      const img = new Image();
      img.src = 'data:image/png;base64,' + b64;
      await img.decode();
      const canvas = document.createElement('canvas');
      canvas.width = img.width;
      canvas.height = img.height;
      const ctx = canvas.getContext('2d')!;
      ctx.drawImage(img, 0, 0);
      const data = ctx.getImageData(0, 0, img.width, img.height).data;
      const at = (x: number, y: number) => {
        const i = (y * img.width + x) * 4;
        return [data[i], data[i + 1], data[i + 2]];
      };
      // The well's fill: a pixel in its top padding, below the corner's radius.
      const fill = at(Math.round(6 * scale), Math.round(10 * scale));
      const isInk = (x: number, y: number) =>
        at(x, y).some((channel, c) => Math.abs(channel - fill[c]) > 24);
      return g.items.map((item) => {
        const x1 = Math.floor((item.textLeft - g.well.x) * scale);
        const y0 = Math.round((item.top - g.well.y) * scale);
        const y1 = Math.min(img.height, Math.round((item.top - g.well.y + g.lineHeight) * scale));
        let inkLeft: number | null = null;
        let inkPixels = 0;
        for (let x = 0; x < x1; x++) {
          for (let y = y0; y < y1; y++) {
            if (!isInk(x, y)) continue;
            inkPixels++;
            if (inkLeft === null || x < inkLeft) inkLeft = x;
          }
        }
        return {
          list: item.list,
          index: item.index,
          clipEdge: g.clipEdge,
          wellLeft: g.well.x,
          textLeft: item.textLeft,
          inkLeft: inkLeft === null ? null : g.well.x + inkLeft / scale,
          inkPixels,
        };
      });
    },
    { b64: shot.toString('base64'), g: geometry, scale: SCALE }
  );
}

describe('list markers in a tool output well (merge-verify #391, screen 20)', () => {
  let browser: Browser | null = null;
  const pages = {} as Record<Variant, { script: string; css: string }>;
  const measured: Partial<Record<Variant, Promise<Measured>>> = {};

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the tool output list marker cases.');
      return;
    }
    for (const variant of ['fixed', 'no-gutter', 'one-digit-only'] as const) {
      pages[variant] = await bundle(variant);
    }
  }, 240_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Mount every case, open each row and its "Show more", and read every strip twice. */
  async function lay(variant: Variant): Promise<Measured> {
    const tab = await browser!.newPage({
      viewport: { width: 1000, height: 2400 },
      deviceScaleFactor: SCALE,
    });
    try {
      const { script, css } = pages[variant];
      await tab.setContent(
        `<!doctype html><html><head><style>${css}</style></head>` +
          `<body style="background:var(--background-default)"><div id="root"></div>` +
          `<script>window.__outputs=${JSON.stringify(Object.entries(OUTPUTS))}</script>` +
          `<script>${script}</script></body></html>`
      );
      await tab.waitForSelector('button.br-transcript-row-head');
      for (const head of await tab.$$('button.br-transcript-row-head')) await head.click();
      await tab.waitForTimeout(500);
      for (const more of await tab.$$('button.br-tool-more')) await more.click();
      // Past the disclosure's open motion, so every well stands at its final size.
      await tab.waitForTimeout(800);
      if (SHOTS) {
        await tab.screenshot({
          path: resolve(SHOTS, `list-markers-${variant}.png`),
          fullPage: true,
        });
      }
      const painted = {} as Record<CaseName, Reading>;
      for (const name of CASES) {
        painted[name] = { items: await readStrips(tab, name), ...(await readLists(tab, name)) };
      }
      // Lift the clip. Still a block formatting context (flow-root), so no margin collapses
      // differently and nothing moves: only what hung past the edge comes back.
      await tab.evaluate(() => {
        document
          .querySelectorAll<HTMLElement>('.br-transcript-row-well .biorouter-markdown')
          .forEach((md) => {
            md.style.setProperty('overflow', 'visible', 'important');
            md.style.setProperty('display', 'flow-root', 'important');
          });
      });
      await tab.waitForTimeout(100);
      const result = {} as Measured;
      for (const name of CASES) {
        result[name] = {
          painted: painted[name],
          unclipped: { items: await readStrips(tab, name), ...(await readLists(tab, name)) },
        };
      }
      return result;
    } finally {
      await tab.close();
    }
  }

  const measure = (variant: Variant) => (measured[variant] ??= lay(variant));

  /** The items whose marker reaches past the clip edge, or loses ink to it. */
  const cut = (c: { painted: Reading; unclipped: Reading }) =>
    c.unclipped.items.filter((item, i) => {
      const painted = c.painted.items[i];
      return (
        item.inkLeft === null ||
        item.inkLeft < item.clipEdge ||
        painted.inkPixels !== item.inkPixels ||
        painted.inkLeft !== item.inkLeft
      );
    });

  it('paints every marker whole and inside the well, in every list shape', async () => {
    if (!browser) return;
    const m = await measure('fixed');
    for (const name of CASES) {
      const c = m[name];
      // Not vacuous: every item was found, lifting the clip moved nothing, and every strip
      // holds its marker.
      expect(c.unclipped.items.length, name).toBeGreaterThan(0);
      expect(
        c.unclipped.items.map((i) => i.textLeft),
        name
      ).toEqual(c.painted.items.map((i) => i.textLeft));
      for (const item of c.unclipped.items) {
        const where = `${name}, list ${item.list} item ${item.index}`;
        expect(item.inkPixels, `${where}: marker ink`).toBeGreaterThan(0);
        expect(item.inkLeft!, `${where}: inside the clip`).toBeGreaterThanOrEqual(item.clipEdge);
        expect(item.inkLeft!, `${where}: inside the well`).toBeGreaterThan(item.wellLeft);
      }
      expect(cut(c), name).toEqual([]);
    }
  });

  it('counts the gutter in mono advances, one more once the numbers reach two digits', async () => {
    if (!browser) return;
    const m = await measure('fixed');
    expect(m.bugReport.painted.gutters).toEqual([3]);
    expect(m.bullets.painted.gutters).toEqual([3]);
    expect(m.twelve.painted.gutters).toEqual([4]);
    expect(m.continued.painted.gutters).toEqual([3, 4]);
    expect(m.startsAtTwelve.painted.gutters).toEqual([4]);
    // A list continued after a code block keeps the half above's text edge while its numbers
    // stay one digit, and bulleted and numbered items share that edge.
    expect(m.continuedShort.painted.gutters).toEqual([3, 3]);
    const [first, rest] = m.continuedShort.painted.textEdges;
    expect(rest).toBe(first);
    expect(m.bullets.painted.textEdges[0]).toBe(m.bugReport.painted.textEdges[0]);
  });

  describe('the instrument', () => {
    it('sees the cut with the gutter rules taken out (the defect as found)', async () => {
      if (!browser) return;
      const m = await measure('no-gutter');
      // "1." read "l.": every one-digit marker of the bug-report method hangs past the edge.
      expect(cut(m.bugReport).map((i) => i.index)).toEqual([0, 1, 2, 3, 4]);
      expect(m.bugReport.painted.items[0].inkPixels).toBeLessThan(
        m.bugReport.unclipped.items[0].inkPixels
      );
      expect(cut(m.twelve)).toHaveLength(12);
    });

    it('sees two-digit numbers cut with only the one-digit gutter kept', async () => {
      if (!browser) return;
      const m = await measure('one-digit-only');
      expect(cut(m.bugReport)).toEqual([]);
      expect(cut(m.twelve).map((i) => i.index)).toEqual([9, 10, 11]);
      expect(cut(m.continued).map((i) => [i.list, i.index])).toEqual([[1, 2]]);
      expect(cut(m.startsAtTwelve)).toHaveLength(3);
    });
  });
});
