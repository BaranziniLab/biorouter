// @vitest-environment node
/**
 * The Dictionary list measured in a real layout engine: jsdom lays nothing out, so only
 * Chromium can say whether the row under the pointer kept its box ("Data and live updates" in
 * `docs/dictation/dictionary.md`), and whether a list at the store's 5,000 row cap draws only
 * the rows near the viewport with every row still in its place ("Page layout").
 *
 * The real `DictionaryList`, `DictionaryRow`, `groupEntries` and `useHoverHold` are bundled with
 * esbuild and painted under the whole stylesheet, Tailwind compiled from the bundle's classes
 * plus the area's own sheet, inside a 600px scroller as the page's `ScrollArea` would hold them.
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
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const ENTRY = `
import * as React from 'react';
import { flushSync } from 'react-dom';
import { createRoot } from 'react-dom/client';
import DictionaryList from './DictionaryList';
import { useHoverHold } from './hoverHold';
import { groupEntries } from './rows';

const row = (written, extra = {}) => ({
  id: 'dct_' + written, rev: 1, written, tier: 'public', status: 'active', source: 'user',
  reviewed: true, spoken_forms: [], misrecognitions: [], kind: null, match: 'fuzzy', bias: 'auto',
  context_cues: [], note: null, reason: null, provenance: null, affiliation: null,
  created_at: '2026-11-01T09:00:00Z', updated_at: '2026-11-01T09:00:00Z', uses: 3,
  last_used_at: '2026-11-04T17:02:11Z', corrected_to: 0, ...extra,
});
const noop = () => {};
const actions = {
  onEdit: noop, onCopy: noop, onMakePublic: noop, onMakePrivate: noop, onDelete: noop,
  onAccept: noop, onDismiss: noop, onKeep: noop, onRemove: noop,
};

function List() {
  const [rows, setRows] = React.useState([row('Charlie'), row('Delta'), row('Echo')]);
  window.__setRows = (names) => setRows(names.map((name) => row(name)));
  // Synchronous, so the caller can time the render, its commit and the list's measuring.
  window.__setMany = (count) =>
    flushSync(() =>
      setRows(
        Array.from({ length: count }, (_, index) =>
          row('GENE' + String(index).padStart(4, '0'), { source: 'import' })
        )
      )
    );
  const hold = useHoverHold(rows);
  const groups = React.useMemo(() => groupEntries(hold.shown), [hold.shown]);
  return (
    <div id="scroller" style={{ height: 600, overflowY: 'auto', width: 808 }}>
      <div style={{ width: 760, padding: 24 }}>
        <DictionaryList
          groups={groups}
          settled={hold.settled}
          readOnly={false}
          tierChangeAllowed
          actions={actions}
          holding={hold.holding}
          handlers={hold.handlers}
        />
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')).render(<List />);
`;

const LIST = '[data-testid="dictionary-groups"]';

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

describe('the Dictionary list in a real layout', { timeout: 90_000 }, () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Dictionary rows test.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"', 'process.env.ALPHA': 'false' },
      loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
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
    css += readFileSync(resolve(here, 'dictionary.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 700 } });
    await tab.route('http://dictionary.test/**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'text/html',
        body:
          `<!doctype html><html><head><style>${css}</style></head>` +
          `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`,
      })
    );
    await tab.goto('http://dictionary.test/');
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-dictionary-row="dct_Delta"]');
    return tab;
  }

  const box = async (tab: Page, id: string) => {
    const found = await tab.locator(`[data-dictionary-row="${id}"]`).boundingBox();
    if (!found) throw new Error(`no row ${id}`);
    return found;
  };

  /** Fill the list with `count` imported rows; answers how long the render and its commit took. */
  const fill = (tab: Page, count: number) =>
    tab.evaluate(async (rows) => {
      const t0 = performance.now();
      (window as unknown as { __setMany: (count: number) => void }).__setMany(rows);
      const committed = performance.now() - t0;
      await new Promise((done) => requestAnimationFrame(() => setTimeout(done)));
      return { committed, painted: performance.now() - t0 };
    }, count);

  /** Scroll the scroller to `top` and wait a frame. */
  const scrollTo = (tab: Page, top: number) =>
    tab.evaluate(async (y) => {
      document.getElementById('scroller')!.scrollTop = y;
      await new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
    }, top);

  /** The imported group's rows list, its first row's height, and how many rows are drawn. */
  const geometry = (tab: Page) =>
    tab.evaluate(() => {
      const scroller = document.getElementById('scroller')!;
      const list = document.querySelector<HTMLElement>(
        '[data-dictionary-section="imported"] [data-dictionary-rows]'
      )!;
      const origin = scroller.getBoundingClientRect().top - scroller.scrollTop;
      const rect = list.getBoundingClientRect();
      const row = list.querySelector<HTMLElement>('[data-dictionary-row]')!;
      return {
        listTop: rect.top - origin,
        listHeight: rect.height,
        pitch: row.getBoundingClientRect().height,
        drawn: document.querySelectorAll('[data-dictionary-row]').length,
        scrollHeight: scroller.scrollHeight,
      };
    });

  /** A drawn row's top in the scroller's content coordinates, or null when it is not drawn. */
  const rowTop = (tab: Page, id: string) =>
    tab.evaluate((rowId) => {
      const scroller = document.getElementById('scroller')!;
      const element = document.querySelector(`[data-dictionary-row="${rowId}"]`);
      if (!element) return null;
      const origin = scroller.getBoundingClientRect().top - scroller.scrollTop;
      return element.getBoundingClientRect().top - origin;
    }, id);

  it('keeps the hovered row where it is while a new term arrives above it', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      const before = await box(tab, 'dct_Delta');
      // The pointer rests on Delta's name, as a person reading the row would.
      await tab.mouse.move(before.x + 40, before.y + before.height / 2);
      await tab.waitForSelector(`${LIST}[data-holding="true"]`);

      await tab.evaluate(() =>
        (window as unknown as { __setRows: (names: string[]) => void }).__setRows([
          'Alpha',
          'Bravo',
          'Charlie',
          'Delta',
          'Echo',
        ])
      );
      await tab.waitForTimeout(150);
      const held = await box(tab, 'dct_Delta');
      expect(held.y).toBe(before.y);
      expect(await tab.locator('[data-dictionary-row="dct_Alpha"]').count()).toBe(0);

      // The pointer leaves, to the right of the 760px column where the list cannot grow
      // into it: the list catches up, and Delta moves down by two rows.
      await tab.mouse.move(880, 20);
      await tab.waitForSelector('[data-dictionary-row="dct_Alpha"]');
      const after = await box(tab, 'dct_Delta');
      expect(after.y).toBeGreaterThan(before.y + before.height);
      expect(await tab.locator(`${LIST}[data-holding="true"]`).count()).toBe(0);
    } finally {
      await tab.close();
    }
  });

  it('shows a new term at once when the pointer is not over the list', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      await tab.mouse.move(880, 20);
      await tab.evaluate(() =>
        (window as unknown as { __setRows: (names: string[]) => void }).__setRows([
          'Alpha',
          'Charlie',
        ])
      );
      await tab.waitForSelector('[data-dictionary-row="dct_Alpha"]');
      expect(await tab.locator('[data-dictionary-row="dct_Delta"]').count()).toBe(0);
    } finally {
      await tab.close();
    }
  });

  it('draws only the rows near the viewport at the 5,000 row cap, each in its place', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      await tab.mouse.move(880, 20);
      const timing = await fill(tab, 5_000);
      const start = await geometry(tab);
      console.info(
        `5,000 rows: render and commit ${timing.committed.toFixed(1)} ms, painted ` +
          `${timing.painted.toFixed(1)} ms, ${start.drawn} rows drawn, pitch ${start.pitch}px`
      );
      // A 600px view of rows about 57px apart, plus 12 lines of overscan below.
      expect(start.drawn).toBeLessThan(40);
      // The placeholders stand for every row not drawn, to the pixel.
      expect(Math.abs(start.listHeight - 5_000 * start.pitch)).toBeLessThan(1);
      // Measured at about 20 ms; the bound leaves room for a loaded machine.
      expect(timing.committed).toBeLessThan(1_000);

      // The middle of the list: row 2,500 is drawn exactly where 2,500 real rows above it put it.
      const middle = start.listTop + 2_500 * start.pitch;
      await scrollTo(tab, middle - 200);
      expect(await rowTop(tab, 'dct_GENE2500')).toBeCloseTo(middle, 0);
      const mid = await geometry(tab);
      expect(mid.drawn).toBeLessThan(50);
      expect(mid.scrollHeight).toBe(start.scrollHeight);

      // The end: the last row's bottom is the list's bottom.
      await scrollTo(tab, start.scrollHeight);
      const lastTop = await rowTop(tab, 'dct_GENE4999');
      expect(lastTop).not.toBeNull();
      expect(lastTop! + start.pitch).toBeCloseTo(start.listTop + start.listHeight, 0);
    } finally {
      await tab.close();
    }
  });

  it('keeps the focused row drawn when the list is scrolled away from it', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      await tab.mouse.move(880, 20);
      await fill(tab, 5_000);
      await tab
        .locator('[data-dictionary-row="dct_GENE0001"] button[data-row-action]')
        .last()
        .focus();
      const start = await geometry(tab);
      await scrollTo(tab, start.listTop + 3_000 * start.pitch);
      expect(await rowTop(tab, 'dct_GENE3000')).not.toBeNull();
      // Still drawn, still focused, still where it was.
      expect(await rowTop(tab, 'dct_GENE0001')).toBeCloseTo(start.listTop + start.pitch, 0);
      expect(
        await tab.evaluate(
          () =>
            document.activeElement
              ?.closest('[data-dictionary-row]')
              ?.getAttribute('data-dictionary-row') ?? null
        )
      ).toBe('dct_GENE0001');
    } finally {
      await tab.close();
    }
  });
});
