// @vitest-environment node
/**
 * A `dictionary__add_terms` row, laid out by Chromium in the 760px chat column (PR-23 visual
 * review).
 *
 * The row's terms (each beside its own Keep and Undo) and Review in Dictionary used to sit on
 * the label's line, where they took the room the label needed: as the last tool row of a message
 * (whose line `message-meta.css` ends 192px short for the hover actions), the collapsed label
 * read "Added te…" and Review in Dictionary wrapped onto a line of its own. They now take a line
 * of their own under the label (`trailingPlacement="below"`, `transcript-row.css`).
 *
 * jsdom lays nothing out, so this bundles the real `ToolCallWithResponse` with esbuild (its
 * component stylesheets included), compiles `styles/main.css` with Tailwind over the bundle's
 * classes, and measures the row. "The instrument" lays the same page out with the two
 * `data-trailing='below'` rules taken back out of `transcript-row.css`, and requires the label to
 * be cut there: a measurement that cannot fail proves nothing. A mutation that matches nothing
 * throws. Set `DICTIONARY_ROW_SHOTS_DIR` to save a screenshot of each page there.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../styles');
const requireFromDesktop = createRequire(resolve(here, '../../package.json'));
const SHOTS = process.env.DICTIONARY_ROW_SHOTS_DIR;
const COLUMN = 760;
/** `message-meta.css`: the last tool row of a message ends this short under its hover actions. */
const ACTIONS_BELOW_PADDING = 192;

type Row = {
  id: string;
  rev?: number;
  written?: string;
  status: 'active' | 'suggested';
  reviewed: boolean;
  source: 'agent';
};

/** One tool row per case, in page order. `short` ends the row's line as `message-meta.css` does. */
const CASES: Record<string, { rows: Row[]; short?: boolean }> = {
  /** Two unreviewed terms: two Keep and Undo pairs, then Review in Dictionary. */
  two: {
    rows: [
      {
        id: 'dct_epas',
        rev: 1,
        written: 'EPAS1',
        status: 'active',
        reviewed: false,
        source: 'agent',
      },
      {
        id: 'dct_hif',
        rev: 2,
        written: 'HIF2A',
        status: 'active',
        reviewed: false,
        source: 'agent',
      },
    ],
  },
  /** The same row as the last tool row of a message, whose line ends 192px short. */
  twoShort: {
    short: true,
    rows: [
      {
        id: 'dct_epas',
        rev: 1,
        written: 'EPAS1',
        status: 'active',
        reviewed: false,
        source: 'agent',
      },
      {
        id: 'dct_hif',
        rev: 2,
        written: 'HIF2A',
        status: 'active',
        reviewed: false,
        source: 'agent',
      },
    ],
  },
  /** A suggested term: Add to dictionary in place of Keep and Undo. */
  suggested: {
    rows: [
      { id: 'dct_vhl', written: 'VHL', status: 'suggested', reviewed: false, source: 'agent' },
    ],
  },
};
type CaseName = keyof typeof CASES;
const NAMES = Object.keys(CASES) as CaseName[];

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import ToolCallWithResponse from './ToolCallWithResponse';

const cases = (window as unknown as { __cases: Array<[string, { rows: unknown[] }]> }).__cases;

function App() {
  return (
    <div style={{ width: ${COLUMN}, margin: '20px 0 0 100px' }}>
      {cases.map(([name, c]) => (
        <div key={name} data-case={name} style={{ marginBottom: 24 }}>
          <ToolCallWithResponse
            isCancelledMessage={false}
            toolRequest={{
              type: 'toolRequest',
              id: name,
              toolCall: {
                status: 'success',
                value: { name: 'dictionary__add_terms', arguments: { terms: [{ written: 'EPAS1' }] } },
              },
            }}
            toolResponse={{
              type: 'toolResponse',
              id: name,
              toolResult: {
                status: 'success',
                value: {
                  content: [
                    { type: 'text', text: 'Added terms to the dictionary.' },
                    {
                      type: 'text',
                      text: JSON.stringify({ dictionary_rows: c.rows }),
                      annotations: { audience: ['user'] },
                    },
                  ],
                },
              },
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

/** The two rules as `transcript-row.css` writes them, to take back out. */
const BELOW_RULES =
  /\.br-transcript-row-line\[data-trailing='below'\] \{[\s\S]*?\}\n\n\.br-transcript-row-line\[data-trailing='below'\] > \.br-transcript-row-trailing \{[\s\S]*?\}\n/;

type Variant = 'fixed' | 'inline';

function mutate(source: string, variant: Variant): string {
  if (variant === 'fixed') return source;
  if (!BELOW_RULES.test(source)) throw new Error("mutation target missing: the 'below' rules");
  return source.replace(BELOW_RULES, '');
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
    outdir: resolve(here, '.dictionary-row-bundle'),
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
  if (!components.includes('.br-transcript-row-line')) {
    throw new Error('the bundle carried no transcript-row.css: the rows would be unstyled');
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

type Box = { left: number; right: number; top: number; bottom: number; height: number };
type Reading = {
  label: { text: string; clipped: boolean; box: Box };
  head: Box;
  line: { right: number; contentRight: number };
  actions: Box | null;
  pairs: Array<{ term: Box | null; buttons: Box[]; termClipped: boolean }>;
  review: Box | null;
};

describe('the dictionary__add_terms row in the 760px column', () => {
  let browser: Browser | null = null;
  const pages = {} as Record<Variant, { script: string; css: string }>;
  const measured: Partial<Record<Variant, Promise<Record<CaseName, Reading>>>> = {};

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the dictionary tool row geometry cases.');
      return;
    }
    for (const variant of ['fixed', 'inline'] as const) pages[variant] = await bundle(variant);
  }, 240_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function lay(variant: Variant): Promise<Record<CaseName, Reading>> {
    const tab = await browser!.newPage({ viewport: { width: 1000, height: 900 } });
    try {
      const { script, css } = pages[variant];
      const short = NAMES.filter((name) => CASES[name].short)
        .map((name) => `[data-case='${name}'] .br-transcript-row-line`)
        .join(', ');
      await tab.setContent(
        `<!doctype html><html><head><style>${css}` +
          `${short} { padding-inline-end: ${ACTIONS_BELOW_PADDING}px; }</style></head>` +
          `<body style="background:var(--background-default)"><div id="root"></div>` +
          `<script>window.__cases=${JSON.stringify(Object.entries(CASES))}</script>` +
          `<script>${script}</script></body></html>`
      );
      await tab.waitForSelector('[data-testid="dictionary-tool-actions"]');
      if (SHOTS) {
        await tab.screenshot({
          path: resolve(SHOTS, `dictionary-row-${variant}.png`),
          fullPage: true,
        });
      }
      return await tab.evaluate((names) => {
        const box = (node: Element | null) => {
          if (!node) return null;
          const r = node.getBoundingClientRect();
          return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, height: r.height };
        };
        const out: Record<string, unknown> = {};
        for (const name of names) {
          const root = document.querySelector(`[data-case='${name}']`)!;
          const label = root.querySelector<HTMLElement>('.br-transcript-row-label')!;
          const line = root.querySelector<HTMLElement>('.br-transcript-row-line')!;
          const lineBox = line.getBoundingClientRect();
          const actions = root.querySelector('[data-testid="dictionary-tool-actions"]');
          const review = Array.from(root.querySelectorAll('button')).find(
            (button) => button.textContent === 'Review in Dictionary'
          );
          out[name] = {
            label: {
              text: label.textContent,
              clipped: label.scrollWidth > label.clientWidth,
              box: box(label),
            },
            head: box(root.querySelector('.br-transcript-row-head')),
            line: {
              right: lineBox.right,
              contentRight: lineBox.right - parseFloat(getComputedStyle(line).paddingRight),
            },
            actions: box(actions),
            pairs: Array.from(root.querySelectorAll('[data-dictionary-term]')).map((pair) => {
              const term = pair.querySelector<HTMLElement>('[data-testid="dictionary-tool-term"]');
              return {
                term: box(term),
                termClipped: term ? term.scrollWidth > term.clientWidth : false,
                buttons: Array.from(pair.querySelectorAll('button')).map((b) => box(b)!),
              };
            }),
            review: box(review ?? null),
          };
        }
        return out as Record<string, Reading>;
      }, NAMES);
    } finally {
      await tab.close();
    }
  }

  const measure = (variant: Variant) => (measured[variant] ??= lay(variant));

  it('keeps every word of the label, with the actions on their own line at its left edge', async () => {
    if (!browser) return;
    const m = await measure('fixed');
    for (const name of NAMES) {
      const c = m[name];
      expect(c.label.text, name).toBeTruthy();
      expect(c.label.clipped, `${name}: "${c.label.text}" is cut`).toBe(false);
      expect(c.actions, name).not.toBeNull();
      // Below the head's 28px line, starting where the label starts.
      expect(c.head.height, name).toBe(28);
      expect(c.actions!.top, name).toBeGreaterThanOrEqual(c.head.bottom - 0.5);
      expect(Math.abs(c.actions!.left - c.label.box.left), name).toBeLessThanOrEqual(0.5);
      // Inside the line's content box, so the hover actions' 192px stay clear.
      expect(c.actions!.right, name).toBeLessThanOrEqual(c.line.contentRight + 0.5);
    }
  });

  it('draws each term on one line with its own buttons, and Review in Dictionary whole', async () => {
    if (!browser) return;
    const m = await measure('fixed');
    for (const name of NAMES) {
      const c = m[name];
      expect(c.pairs.length, name).toBe(CASES[name].rows.length);
      for (const pair of c.pairs) {
        expect(pair.term, name).not.toBeNull();
        expect(pair.termClipped, name).toBe(false);
        const middle = (b: Box) => (b.top + b.bottom) / 2;
        for (const button of pair.buttons) {
          expect(Math.abs(middle(button) - middle(pair.term!)), name).toBeLessThanOrEqual(1);
          expect(button.left, name).toBeGreaterThanOrEqual(pair.term!.right);
        }
      }
      // One line of text in a 24px control, on the line the last term's buttons sit on.
      expect(c.review, name).not.toBeNull();
      expect(c.review!.height, name).toBe(24);
      const last = c.pairs[c.pairs.length - 1].buttons[0];
      expect(Math.abs(c.review!.top - last.top), name).toBeLessThanOrEqual(1);
    }
  });

  it('the instrument: with the rules taken out, the defect as found comes back', async () => {
    if (!browser) return;
    const m = await measure('inline');
    // As the last tool row of a message: "Added te…", and Review in Dictionary on a line of its
    // own below the last term's buttons.
    const c = m.twoShort;
    expect(c.label.clipped).toBe(true);
    const last = c.pairs[c.pairs.length - 1].buttons[0];
    expect(c.review!.top).toBeGreaterThan(last.bottom - 0.5);
  });
});
