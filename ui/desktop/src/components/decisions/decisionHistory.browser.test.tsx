// @vitest-environment node
/**
 * The decision history view's list over 2,000 calls, scrolled in Chromium with the whole
 * stylesheet.
 *
 * jsdom lays nothing out, so `DecisionHistory.test.tsx` can say which rows are drawn and never
 * whether the windowing holds when a real list scrolls. This file bundles the real
 * `DecisionHistory` with esbuild, answers its requests from 2,000 synthetic rows through the
 * generated client's own `fetch` hook (no network), mounts it at a side panel's size, and
 * scrolls to the end: every page must load, the rows stay 44px, and the number of rows in the
 * DOM stays bounded however many have loaded.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type BrowserServer } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { fontScale } from '../../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHEETS = ['decisionsPanel.css', 'probabilityBar.css']
  .map((name) => readFileSync(resolve(here, name), 'utf8'))
  .join('\n');

const TOTAL = 2000;
const PANEL = { width: 480, height: 760 };

/**
 * Each page sets `window.SCENARIO` before the bundle runs: how many calls, the panel's size,
 * the summary the filters read, and the filter the view opens with.
 */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { client } from '../../api/client.gen';
import DecisionHistory from './DecisionHistory';

const SCENARIO = window.SCENARIO || {};
const TOTAL = SCENARIO.total ?? ${TOTAL};
const row = (i) => ({
  id: 'd' + i, sessionId: 's1',
  createdAt: new Date(Date.now() - i * 1000).toISOString(),
  origin: 'tool', originId: 'call-1', label: 'decide', turnId: 't1', batchId: 'b1',
  chunkIndex: i, chunkId: 'note-' + String(i + 1).padStart(4, '0'),
  model: { provider: 'ollama_decision', model: 'tev1:0.8b' }, servedModel: null,
  tier: 'private', calibrated: true,
  questions: { has_ms: { instructions: 'Does this note state the patient has multiple sclerosis?', kind: { binary: {} }, threshold: { yes: 0.9 } } },
  decisions: [{ question: 'has_ms', answer: { type: 'binary', value: i % 3 === 0 },
    probabilities: { yes: i % 3 === 0 ? 0.95 : 0.2, no: i % 3 === 0 ? 0.05 : 0.8 },
    confidence: 0.6, passed: i % 3 === 0, passProbability: i % 3 === 0 ? 0.95 : 0.2,
    source: { kind: 'native' }, calibrated: true, model: { provider: 'ollama_decision', model: 'tev1:0.8b' },
    tier: 'private', requestId: 'r' + i, latencyMs: 22, usage: { inputTokens: 400, outputTokens: 1 }, raw: {} }],
  outcome: i % 3 === 0 ? 'passed' : 'failed', contextPreview: 'Neuro f/u ' + i,
  contextBytes: 2048, contextTruncated: false, error: null, latencyMs: 22,
  inputTokens: 400, outputTokens: 1, privacyEnforced: true,
});
window.REQUESTS = [];
const json = (body) => new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
client.setConfig({
  baseUrl: 'http://decisions.test',
  fetch: async (request) => {
    const url = new URL(request.url);
    window.REQUESTS.push(url.pathname + url.search);
    if (url.pathname.endsWith('/decisions/summary')) {
      return json(SCENARIO.summary ?? { total: TOTAL, aggregates: [], recentTurns: [] });
    }
    const detail = url.pathname.match(/\\/decisions\\/(d\\d+)$/);
    if (detail) return json({ record: row(Number(detail[1].slice(1))), context: { text: 'Full note ' + detail[1] } });
    const start = Number(url.searchParams.get('before') || 0);
    const limit = Number(url.searchParams.get('limit') || 100);
    const end = Math.min(TOTAL, start + limit);
    const records = [];
    for (let i = start; i < end; i += 1) records.push(row(i));
    return json({ records, nextCursor: end < TOTAL ? String(end) : null, total: TOTAL });
  },
});

function Page() {
  const panel = SCENARIO.panel ?? { width: ${PANEL.width}, height: ${PANEL.height} };
  return (
    <div data-testid="panel" style={{ position: 'relative', width: panel.width + 'px', height: panel.height + 'px' }}>
      <DecisionHistory sessionId="s1" filter={SCENARIO.filter ?? {}} filterKey={1} />
    </div>
  );
}
createRoot(document.getElementById('root')).render(<Page />);
`;

const TEARDOWN_STEP_MS = 5_000;

const settledWithin = async (work: Promise<unknown>, ms: number): Promise<boolean> => {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<false>((done) => {
    timer = setTimeout(() => done(false), ms);
    (timer as unknown as { unref?: () => void }).unref?.();
  });
  try {
    return (
      (await Promise.race([
        work.then(
          () => true,
          () => true
        ),
        expired,
      ])) !== false
    );
  } finally {
    if (timer) clearTimeout(timer);
  }
};

const launchChromium = async (): Promise<{ server: BrowserServer; browser: Browser } | null> => {
  for (const options of [{}, { channel: 'chromium' }, { channel: 'chrome' }] as const) {
    let server: BrowserServer | null = null;
    try {
      server = await chromium.launchServer(options);
      return { server, browser: await chromium.connect(server.wsEndpoint()) };
    } catch {
      if (server) await settledWithin(server.close(), TEARDOWN_STEP_MS);
    }
  }
  return null;
};

describe('the decision history list over 2,000 calls, in a real layout engine', () => {
  let browser: Browser | null = null;
  let server: BrowserServer | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    const launched = await launchChromium();
    browser = launched?.browser ?? null;
    server = launched?.server ?? null;
    if (!browser) {
      console.warn('No Chromium found: skipping the decision history tests.');
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
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    const tailwind = compiler.build(
      new Scanner({}).scanFiles([{ content: script, extension: 'js' }])
    );
    css = (await postcss([fontScale()]).process(`${tailwind}\n${SHEETS}`, { from: undefined })).css;
  }, 180_000);

  afterAll(async () => {
    const connection = browser;
    const launched = server;
    browser = null;
    server = null;
    if (connection) await settledWithin(connection.close(), TEARDOWN_STEP_MS);
    if (!launched) return;
    if (await settledWithin(launched.close(), TEARDOWN_STEP_MS)) return;
    launched.process().kill('SIGKILL');
    await settledWithin(launched.close(), TEARDOWN_STEP_MS);
  }, 30_000);

  /** A page with the whole stylesheet, running the bundle under `scenario`. */
  async function openPage(
    scenario: Record<string, unknown>,
    viewport = { width: 1000, height: 900 },
    dark = false
  ) {
    const tab = await browser!.newPage({ viewport });
    tab.setDefaultTimeout(10_000);
    await tab.setContent(
      `<!doctype html><html class="${dark ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div>` +
        `<script>window.SCENARIO = ${JSON.stringify(scenario)};</script></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-testid="decision-row"]');
    return tab;
  }

  const TURNS = {
    total: 10,
    aggregates: [],
    recentTurns: [
      {
        turnId: 'turn-a',
        origin: 'tool',
        originId: 'call-1',
        label: 'decide',
        count: 6,
        passed: 2,
        failed: 4,
        errors: 0,
        firstAt: new Date().toISOString(),
        lastAt: new Date().toISOString(),
        lastPassProbability: 0.2,
      },
      {
        turnId: 'turn-b',
        origin: 'tool',
        originId: 'call-1',
        label: 'decide',
        count: 4,
        passed: 1,
        failed: 3,
        errors: 0,
        firstAt: new Date(Date.now() - 60_000).toISOString(),
        lastAt: new Date(Date.now() - 60_000).toISOString(),
        lastPassProbability: 0.2,
      },
    ],
  };

  it('hugs ten calls in a narrow panel: no empty band above the detail (A-5)', async () => {
    if (!browser) return;
    const tab = await openPage(
      { total: 10, panel: { width: 600, height: 1200 } },
      { width: 1000, height: 1400 }
    );
    await tab.waitForSelector('[data-testid="decision-detail"]');
    const box = await tab.evaluate(() => {
      const pane = document
        .querySelector<HTMLElement>('.br-dh__list-pane')!
        .getBoundingClientRect();
      const detail = document.querySelector<HTMLElement>('.br-dh__detail')!.getBoundingClientRect();
      return { height: pane.height, gap: detail.top - pane.bottom };
    });
    await tab.close();
    expect(box.height).toBe(440);
    expect(box.gap).toBeGreaterThanOrEqual(0);
    expect(box.gap).toBeLessThanOrEqual(8);
  }, 60_000);

  it('keeps the Filters popover open while a turn is picked in its nested menu, and Escape closes one layer at a time', async () => {
    if (!browser) return;
    const tab = await openPage({ total: 10, summary: TURNS });
    await tab.click('[data-testid="decision-filter-more"]');
    await tab.waitForSelector('[data-testid="decision-filters-more"]');
    await tab.click('[data-testid="decision-filter-turn"]');
    await tab.click('[role="menuitemradio"]:nth-child(2)');
    await tab.waitForFunction(() =>
      (window as unknown as { REQUESTS: string[] }).REQUESTS.some((path) =>
        path.includes('turnId=turn-a')
      )
    );
    // The pick applied and the popover stayed open.
    expect(await tab.isVisible('[data-testid="decision-filters-more"]')).toBe(true);
    expect(await tab.textContent('[data-testid="decision-filter-more"]')).toContain(
      'Filters: turn'
    );

    // Escape with the Turn menu open (by key) closes only the menu, back on the Turn chip.
    await tab.focus('[data-testid="decision-filter-turn"]');
    await tab.keyboard.press('Enter');
    await tab.waitForSelector('[role="menu"]');
    await tab.keyboard.press('Escape');
    await tab.waitForSelector('[role="menu"]', { state: 'detached' });
    const first = await tab.evaluate(() => ({
      focus: document.activeElement?.getAttribute('data-testid'),
      popover: !!document.querySelector('[data-testid="decision-filters-more"]'),
    }));
    expect(first).toEqual({ focus: 'decision-filter-turn', popover: true });
    // A second Escape closes the popover, back on Filters.
    await tab.keyboard.press('Escape');
    await tab.waitForSelector('[data-testid="decision-filters-more"]', { state: 'detached' });
    const second = await tab.evaluate(() => document.activeElement?.getAttribute('data-testid'));
    await tab.close();
    expect(second).toBe('decision-filter-more');
  }, 60_000);

  it('returns focus to the Turn chip when Escape closes a menu the pointer opened', async () => {
    if (!browser) return;
    const tab = await openPage({ total: 10, summary: TURNS });
    await tab.click('[data-testid="decision-filter-more"]');
    await tab.waitForSelector('[data-testid="decision-filters-more"]');
    // Opened with the pointer: the trigger never takes focus (it opens on pointerdown). A pick
    // first, so the second opening starts from wherever a pointer pick leaves focus.
    await tab.click('[data-testid="decision-filter-turn"]');
    await tab.click('[role="menuitemradio"]:nth-child(2)');
    await tab.waitForSelector('[role="menu"]', { state: 'detached' });
    await tab.click('[data-testid="decision-filter-turn"]');
    await tab.waitForSelector('[role="menu"]');
    await tab.hover('[role="menuitemradio"]:nth-child(1)');
    await tab.keyboard.press('Escape');
    await tab.waitForSelector('[role="menu"]', { state: 'detached' });
    const state = await tab.evaluate(() => ({
      focus: document.activeElement?.getAttribute('data-testid') ?? document.activeElement?.tagName,
      popover: !!document.querySelector('[data-testid="decision-filters-more"]'),
    }));
    await tab.close();
    expect(state).toEqual({ focus: 'decision-filter-turn', popover: true });
  }, 60_000);

  it('leaves focus alone when a pointer pick closes the Turn menu', async () => {
    if (!browser) return;
    const tab = await openPage({ total: 10, summary: TURNS });
    await tab.click('[data-testid="decision-filter-more"]');
    await tab.waitForSelector('[data-testid="decision-filters-more"]');
    await tab.click('[data-testid="decision-filter-turn"]');
    await tab.click('[role="menuitemradio"]:nth-child(2)');
    await tab.waitForSelector('[role="menu"]', { state: 'detached' });
    const focus = await tab.evaluate(
      () => document.activeElement?.getAttribute('data-testid') ?? null
    );
    await tab.close();
    // The shared pointer-pick rule: no focus ring is painted onto a chip the person only clicked.
    expect(focus).not.toBe('decision-filter-turn');
  }, 60_000);

  it('in dark mode, rests the popover’s engaged chip and hovered field on the popover’s own strong step', async () => {
    if (!browser) return;
    const tab = await openPage(
      { total: 10, summary: TURNS, filter: { origin: 'tool', turnId: 'turn-a' } },
      { width: 1000, height: 900 },
      true
    );
    await tab.click('[data-testid="decision-filter-more"]');
    await tab.waitForSelector('[data-testid="decision-filters-more"]');
    await tab.hover('[data-testid="decision-filter-min-p"]');
    await tab.waitForTimeout(300);
    const colours = await tab.evaluate(() => {
      const probe = (host: Element) => {
        const element = document.createElement('div');
        element.style.background = 'var(--background-fill-strong)';
        host.appendChild(element);
        const value = getComputedStyle(element).backgroundColor;
        element.remove();
        return value;
      };
      const surface = document
        .querySelector('[data-testid="decision-filters-more"]')!
        .closest('.biorouter-popover-surface')!;
      return {
        popoverStep: probe(surface),
        pageStep: probe(document.querySelector('[data-testid="decision-filters"]')!),
        turnChip: getComputedStyle(document.querySelector('[data-testid="decision-filter-turn"]')!)
          .backgroundColor,
        sourceChip: getComputedStyle(
          document.querySelector('[data-testid="decision-filter-source"]')!
        ).backgroundColor,
        field: getComputedStyle(document.querySelector('[data-testid="decision-filter-min-p"]')!)
          .backgroundColor,
      };
    });
    await tab.close();
    expect(colours.popoverStep).not.toBe(colours.pageStep);
    expect(colours.turnChip).toBe(colours.popoverStep);
    expect(colours.sourceChip).toBe(colours.pageStep);
    expect(colours.field).toBe(colours.popoverStep);
  }, 60_000);

  it('scrolls to the 2,000th call a page at a time, drawing a bounded window of 44px rows', async () => {
    if (!browser) return;
    const tab = await browser.newPage({ viewport: { width: 1000, height: 900 } });
    tab.setDefaultTimeout(10_000);
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-testid="decision-row"]');
    await tab.waitForSelector('[data-testid="decision-detail"]');
    // A picture for a person reviewing the layout: `DECISIONS_SCREENSHOT_DIR=/tmp/x`.
    const shots = process.env.DECISIONS_SCREENSHOT_DIR;
    if (shots) {
      await tab.click('[data-testid="decision-detail"] .biorouter-disclosure-trigger');
      await tab.waitForSelector(
        '[data-testid="decision-context"] .br-dh-context__prose:not([data-preview])'
      );
      await tab.screenshot({
        path: `${shots}/decision-history.png`,
        clip: { x: 0, y: 0, ...PANEL },
      });
      await tab.click('[data-testid="decision-filter-more"]');
      await tab.waitForSelector('[data-testid="decision-filters-more"]');
      await tab.screenshot({
        path: `${shots}/decision-history-filters.png`,
        clip: { x: 0, y: 0, ...PANEL },
      });
      await tab.keyboard.press('Escape');
    }

    const first = await tab.evaluate(() => {
      const list = document.querySelector<HTMLElement>('[data-testid="decision-list"]')!;
      const rows = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid="decision-row"]')
      );
      return {
        clientHeight: list.clientHeight,
        drawn: rows.length,
        heights: [...new Set(rows.map((row) => Math.round(row.getBoundingClientRect().height)))],
        rowRight: Math.max(...rows.map((row) => row.getBoundingClientRect().right)),
        listRight: list.getBoundingClientRect().right,
      };
    });
    expect(first.clientHeight).toBeGreaterThan(100);
    expect(first.heights).toEqual([44]);
    expect(first.rowRight).toBeLessThanOrEqual(first.listRight + 0.5);
    const bound = Math.ceil(first.clientHeight / 44) + 2 * 8 + 2;
    expect(first.drawn).toBeLessThanOrEqual(bound);

    // Scroll to the end, again and again, until every page has loaded.
    let maxDrawn = first.drawn;
    for (let step = 0; step < 60; step += 1) {
      const state = await tab.evaluate(async () => {
        const list = document.querySelector<HTMLElement>('[data-testid="decision-list"]')!;
        list.scrollTop = list.scrollHeight;
        list.dispatchEvent(new Event('scroll'));
        await new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
        await new Promise((done) => setTimeout(done, 30));
        return {
          drawn: document.querySelectorAll('[data-testid="decision-row"]').length,
          last: Array.from(document.querySelectorAll('[data-testid="decision-row"]')).map((row) =>
            row.getAttribute('data-decision-id')
          ),
          scrollHeight: list.scrollHeight,
        };
      });
      maxDrawn = Math.max(maxDrawn, state.drawn);
      if (state.last.includes(`d${TOTAL - 1}`)) break;
    }
    const end = await tab.evaluate(() => {
      const list = document.querySelector<HTMLElement>('[data-testid="decision-list"]')!;
      const rows = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid="decision-row"]')
      );
      const box = list.getBoundingClientRect();
      const lastRow = rows.find((row) => row.getAttribute('data-decision-id') === 'd1999');
      return {
        ids: rows.map((row) => row.getAttribute('data-decision-id')),
        scrollHeight: list.scrollHeight,
        lastVisible: lastRow
          ? lastRow.getBoundingClientRect().bottom <= box.bottom + 1 &&
            lastRow.getBoundingClientRect().top >= box.top - 1
          : false,
        pages: (window as unknown as { REQUESTS: string[] }).REQUESTS.filter(
          (path) => path === '/sessions/s1/decisions' || path.startsWith('/sessions/s1/decisions?')
        ).length,
        pageScrollWidth: document.documentElement.scrollWidth,
      };
    });
    await tab.close();
    expect(end.ids).toContain('d1999');
    expect(end.lastVisible).toBe(true);
    // Every row of 2,000 has its place, and no more than one window of them is drawn.
    expect(end.scrollHeight).toBe(TOTAL * 44);
    expect(end.pages).toBe(TOTAL / 100);
    expect(maxDrawn).toBeLessThanOrEqual(bound + 1);
    expect(end.pageScrollWidth).toBeLessThanOrEqual(1000);
  }, 120_000);
});
