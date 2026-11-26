// @vitest-environment node
/**
 * The **History** tab's list measured in a real layout engine: jsdom lays nothing out, so only
 * Chromium can say whether every collapsed row has one height whatever its text, whether an
 * expanded row stays drawn (and every row below it in its place) while the list is scrolled away
 * from it, whether the row under the pointer keeps its box while a dictation arrives, and where
 * the lock badge is drawn (`history-design.md` 10.2).
 *
 * The real `HistoryList`, `HistoryRow`, `groupDictationsByDate` and `useHoverHold` are bundled
 * with esbuild and painted under the whole stylesheet, Tailwind compiled from the bundle's
 * classes plus the area's own sheet, inside a 600px scroller as the page's `ScrollArea` holds
 * them.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../../package.json'));

const ENTRY = `
import * as React from 'react';
import { flushSync } from 'react-dom';
import { createRoot } from 'react-dom/client';
import HistoryList from './HistoryList';
import { groupDictationsByDate } from './historyRows';
import { useHoverHold } from '../hoverHold';

const NOW = Date.now();
const LONG =
  'Query the SPOKE knowledge graph for every drug that targets EPAS1 or EGLN1, then list the ' +
  'trials that enrolled people living above 3,000 metres, with their outcomes and the year each ' +
  'one closed, and say which of them measured haemoglobin before and after the intervention.';
const row = (id, minutesAgo, extra = {}) => ({
  id: 'dch_' + id,
  created_at: new Date(Math.floor((NOW - minutesAgo * 60000) / 1000) * 1000).toISOString(),
  text: 'Dictation ' + id,
  tier: 'public',
  surface: 'composer',
  duration_ms: 14000,
  model: 'whisper-large-v3-turbo-q5_0',
  model_name: 'Large v3 Turbo',
  ...extra,
});
const noop = () => {};
const keyOf = (entry) => entry.id;

function List() {
  const [rows, setRows] = React.useState([
    row('Short', 1, { text: 'EPAS1' }),
    row('Long', 2, { text: LONG, tier: 'private' }),
    row('Delta', 3),
    row('Echo', 4, { tier: 'private' }),
  ]);
  const [expanded, setExpanded] = React.useState(() => new Set());
  window.__arrive = () => setRows((current) => [row('New', 0), ...current]);
  window.__append = () => setRows((current) => [...current, row('Later', 900)]);
  window.__setMany = (count) =>
    flushSync(() =>
      setRows(
        Array.from({ length: count }, (_, index) =>
          row('N' + String(index).padStart(4, '0'), 1 + index * 0.01, {
            text: index % 3 === 0 ? LONG : 'Dictation ' + index,
            tier: index % 2 === 0 ? 'private' : 'public',
          })
        )
      )
    );
  const hold = useHoverHold(rows, { keyOf });
  const groups = React.useMemo(() => groupDictationsByDate(hold.shown, NOW), [hold.shown]);
  const actions = React.useMemo(
    () => ({
      onCopy: noop,
      onOpenChat: noop,
      onMakePublic: noop,
      onMakePrivate: noop,
      onDelete: noop,
      onToggle: (id) =>
        setExpanded((current) => {
          const next = new Set(current);
          if (next.has(id)) next.delete(id);
          else next.add(id);
          return next;
        }),
    }),
    []
  );
  return (
    <div id="scroller" style={{ height: 600, overflowY: 'auto', width: 808 }}>
      <div style={{ width: 760, padding: 24 }}>
        <HistoryList
          groups={groups}
          settled={hold.settled}
          expanded={expanded}
          writeAllowed
          tierChangeAllowed
          actions={actions}
          now={NOW}
          holding={hold.holding}
          handlers={hold.handlers}
        />
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')).render(<List />);
`;

const LIST = '[data-testid="history-groups"]';

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

describe('the History list in a real layout', { timeout: 90_000 }, () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the History rows test.');
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
    css += readFileSync(resolve(here, '../dictionary.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 700 } });
    await tab.route('http://history.test/**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'text/html',
        body:
          `<!doctype html><html><head><style>${css}</style></head>` +
          `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`,
      })
    );
    await tab.goto('http://history.test/');
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-history-row="dch_Delta"]');
    return tab;
  }

  const box = async (tab: Page, id: string) => {
    const found = await tab.locator(`[data-history-row="${id}"]`).boundingBox();
    if (!found) throw new Error(`no row ${id}`);
    return found;
  };

  const scrollTo = (tab: Page, top: number) =>
    tab.evaluate(async (y) => {
      document.getElementById('scroller')!.scrollTop = y;
      await new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
    }, top);

  /** A drawn row's top in the scroller's content coordinates, or null when it is not drawn. */
  const rowTop = (tab: Page, id: string) =>
    tab.evaluate((rowId) => {
      const scroller = document.getElementById('scroller')!;
      const element = document.querySelector(`[data-history-row="${rowId}"]`);
      if (!element) return null;
      const origin = scroller.getBoundingClientRect().top - scroller.scrollTop;
      return element.getBoundingClientRect().top - origin;
    }, id);

  /** The Today group's rows list, a collapsed row's height, and how many rows are drawn. */
  const geometry = (tab: Page) =>
    tab.evaluate(() => {
      const scroller = document.getElementById('scroller')!;
      const list = document.querySelector<HTMLElement>(
        '[data-history-section="today"] [data-history-rows]'
      )!;
      const origin = scroller.getBoundingClientRect().top - scroller.scrollTop;
      const rect = list.getBoundingClientRect();
      const row = list.querySelector<HTMLElement>('[data-history-row]:not([data-expanded])')!;
      return {
        listTop: rect.top - origin,
        listHeight: rect.height,
        pitch: row.getBoundingClientRect().height,
        drawn: document.querySelectorAll('[data-history-row]').length,
        scrollHeight: scroller.scrollHeight,
      };
    });

  it('draws every collapsed row at one height, whatever its text, and grows only when expanded', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      await tab.mouse.move(880, 20);
      const heights = await Promise.all(
        ['dch_Short', 'dch_Long', 'dch_Delta', 'dch_Echo'].map(
          async (id) => (await box(tab, id)).height
        )
      );
      for (const height of heights) expect(height).toBeCloseTo(heights[0], 1);
      // Two lines of 14px body text plus one muted line, and the row's padding.
      expect(heights[0]).toBeGreaterThan(60);
      // The long text is clamped: its paragraph is cut at two lines.
      const clamped = await tab.evaluate(() => {
        const text = document.querySelector<HTMLElement>(
          '[data-history-row="dch_Long"] [data-testid="history-row-text"]'
        )!;
        return { client: text.clientHeight, scroll: text.scrollHeight };
      });
      expect(clamped.scroll).toBeGreaterThan(clamped.client);

      await tab.locator('[data-history-row="dch_Long"] [data-testid="history-row-body"]').click();
      await tab.waitForSelector('[data-history-row="dch_Long"][data-expanded]');
      expect((await box(tab, 'dch_Long')).height).toBeGreaterThan(heights[0] + 10);
      expect((await box(tab, 'dch_Delta')).height).toBeCloseTo(heights[0], 1);
    } finally {
      await tab.close();
    }
  });

  it('draws the lock badge on private rows only', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      const badges: Array<{ id: string; tier: string | null; lock: boolean; name: string | null }> =
        await tab.evaluate(() =>
          [...document.querySelectorAll<HTMLElement>('[data-history-row]')].map((row) => ({
            id: row.dataset.historyRow ?? '',
            tier: row.getAttribute('data-history-tier'),
            lock: row.querySelector('.br-icon-lock-badge, [class*="-private"]') !== null,
            name: row.querySelector('[role="img"]')?.getAttribute('aria-label') ?? null,
          }))
        );
      for (const found of badges) {
        expect(found.lock, found.id).toBe(found.tier === 'private');
        expect(found.name, found.id).toBe(
          found.tier === 'private' ? 'Private dictation' : 'Public dictation'
        );
      }
      expect(badges.some((found) => found.lock)).toBe(true);
      expect(badges.some((found) => !found.lock)).toBe(true);
    } finally {
      await tab.close();
    }
  });

  it('keeps the hovered row where it is while a dictation arrives above it, and lets the next page through', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      const before = await box(tab, 'dch_Delta');
      await tab.mouse.move(before.x + 40, before.y + before.height / 2);
      await tab.waitForSelector(`${LIST}[data-holding="true"]`);

      // A page read at the end moves nothing above it, so it shows at once, under the pointer.
      await tab.evaluate(() => (window as unknown as { __append: () => void }).__append());
      await tab.waitForSelector('[data-history-row="dch_Later"]');
      expect((await box(tab, 'dch_Delta')).y).toBe(before.y);

      // A new dictation would push every row down: it waits until the pointer leaves.
      await tab.evaluate(() => (window as unknown as { __arrive: () => void }).__arrive());
      await tab.waitForTimeout(150);
      expect((await box(tab, 'dch_Delta')).y).toBe(before.y);
      expect(await tab.locator('[data-history-row="dch_New"]').count()).toBe(0);

      await tab.mouse.move(880, 20);
      await tab.waitForSelector('[data-history-row="dch_New"]');
      expect((await box(tab, 'dch_Delta')).y).toBeGreaterThan(before.y + before.height - 1);
    } finally {
      await tab.close();
    }
  });

  it('keeps an expanded row drawn while scrolled away, with every row below it in its place', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      await tab.mouse.move(880, 20);
      await tab.evaluate(() =>
        (window as unknown as { __setMany: (n: number) => void }).__setMany(3_000)
      );
      await tab.waitForSelector('[data-history-row="dch_N0000"]');
      const start = await geometry(tab);
      // A 600px view of rows about 77px apart, plus the overscan: far fewer than 3,000.
      expect(start.drawn).toBeLessThan(40);
      expect(Math.abs(start.listHeight - 3_000 * start.pitch)).toBeLessThan(1);

      await tab.locator('[data-history-row="dch_N0003"] [data-testid="history-row-body"]').click();
      await tab.waitForSelector('[data-history-row="dch_N0003"][data-expanded]');
      const expandedHeight = (await box(tab, 'dch_N0003')).height;
      const extra = expandedHeight - start.pitch;
      expect(extra).toBeGreaterThan(10);

      // Far down the list: row 2,000 sits where 2,000 rows above it, one of them expanded, put it.
      const target = start.listTop + 2_000 * start.pitch + extra;
      await scrollTo(tab, target - 200);
      await tab.waitForSelector('[data-history-row="dch_N2000"]');
      expect(await rowTop(tab, 'dch_N2000')).toBeCloseTo(target, 0);
      // The expanded row is still drawn, still expanded, still where it was.
      expect(await rowTop(tab, 'dch_N0003')).toBeCloseTo(start.listTop + 3 * start.pitch, 0);
      expect(await tab.locator('[data-history-row="dch_N0003"][data-expanded]').count()).toBe(1);
      const after = await geometry(tab);
      expect(after.drawn).toBeLessThan(50);
      expect(Math.abs(after.listHeight - (3_000 * start.pitch + extra))).toBeLessThan(1);

      // The end: the last row's bottom is the list's bottom.
      await scrollTo(tab, after.scrollHeight);
      const lastTop = await rowTop(tab, 'dch_N2999');
      expect(lastTop).not.toBeNull();
      expect(lastTop! + start.pitch).toBeCloseTo(after.listTop + after.listHeight, 0);
    } finally {
      await tab.close();
    }
  });
});
