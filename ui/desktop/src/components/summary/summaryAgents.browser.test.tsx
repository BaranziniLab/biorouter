// @vitest-environment node
/**
 * The subagents in the Chat summary, measured in Chromium with the whole stylesheet.
 *
 * jsdom lays nothing out, so `SummarySubagents.test.tsx` can say which rows exist and never how
 * tall they are, whether a long list scrolls inside the card, or whether a long name pushes the
 * card sideways. This file bundles the real `ChatSummaryRail` (with `ChatSummary` and
 * `SummarySubagents` inside it) with esbuild, mounts it in the rail's own grid at its narrowest,
 * a 240px column (a 228px card after the cell's 12px right padding), compiles `styles/main.css`
 * with Tailwind over the classes the bundle uses, adds the summary's three stylesheets
 * unlayered as the app does, and measures.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type BrowserServer, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { fontScale } from '../../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHEETS = ['chatSummary.css', 'summaryRail.css', 'summaryAgents.css']
  .map((name) => readFileSync(resolve(here, name), 'utf8'))
  .join('\n');

const RAIL_WIDTH = 240;
const PANE_HEIGHT = 720;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ChatSummaryRail } from './ChatSummaryRail';
import { parseSessionSubagents } from '../../utils/sessionSubagents';

const params = window.PARAMS;
const statuses = ['running', 'running', 'queued', 'blocked', 'failed', 'completed'];
const rows = Array.from({ length: params.agents }, (_, i) => ({
  sessionId: 'child-' + i,
  name: 'Subagent ' + (i + 1) + ': read every trial report in the cohort and tabulate the dosing schedules',
  role: i % 3 === 0 ? 'Reader of the very long trial registry exports ' + i : null,
  phaseId: params.plan ? (i < 4 ? 'plan' : 'research') : null,
  status: statuses[i % statuses.length],
  loopPhase: null,
  quietSeconds: i % 7 === 0 ? 600 : 3,
  createdAt: '2026-11-04T10:00:00Z',
  updatedAt: '2026-11-04T10:01:00Z',
  endedAt: null,
  elapsedSeconds: 3900 + i,
  providerName: 'anthropic',
  modelName: params.model ?? 'claude-sonnet-4-5-20250929-with-a-long-suffix',
  extensions: ['developer', 'webdocuments', 'memory'],
  skills: ['scientific-research'],
  knowledgeBases: ['soul'],
  privacyTier: i % 4 === 0 ? 'private' : 'public',
  totalTokens: 123456,
  messageCount: 12,
  toolCallCount: 30,
  resultLine: params.markdownResult
    ? '| Patient | sodium (Na) | potassium (K) | creatinine |'
    : null,
}));
const plan = params.plan
  ? {
      title: 'Survey every trial of the drug across the registries we can reach',
      goal: null,
      revision: 1,
      phases: [
        { id: 'plan', title: 'Plan the search strategy and the inclusion criteria', kind: 'plan',
          status: 'completed', dependsOn: [], agentIds: rows.slice(0, 4).map((r) => r.sessionId),
          counts: { total: 4, done: 4, running: 0, blocked: 0, failed: 0, quiet: 0 },
          gate: params.gate
            ? { threshold: 0.8, questionCount: 2, attempts: 1, maxAttempts: 3, lastPassed: true,
                lastMinP: 0.9, questions: { values_listed: 'Are the values listed?' } }
            : null,
          handoff: null, startedAt: null, finishedAt: null },
        { id: 'research', title: 'Read the trials', kind: 'research', status: 'in_progress',
          dependsOn: ['plan'], agentIds: rows.slice(4).map((r) => r.sessionId),
          counts: { total: rows.length - 4, done: 3, running: 10, blocked: 2, failed: 4, quiet: 3 },
          handoff: null, startedAt: null, finishedAt: null },
        { id: 'verify', title: 'Check the numbers', kind: 'verification', status: 'pending',
          dependsOn: ['research'], agentIds: [],
          counts: { total: 0, done: 0, running: 0, blocked: 0, failed: 0, quiet: 0 },
          handoff: null, startedAt: null, finishedAt: null },
      ],
    }
  : null;
const data = parseSessionSubagents({ parentSessionId: 'parent', subagents: rows, plan });
const summary = {
  toolCalls: '42',
  billedTokens: '211k',
  artifacts: '3',
  codeDelta: { added: 120, removed: 14 },
  todos: {
    items: [
      { id: '1', text: 'Delegate the reading', status: 'completed' },
      { id: '2', text: 'Reconcile the tables', status: 'in_progress' },
    ],
    loading: false, error: false, refresh: () => {},
  },
  subagents: { data, loading: false, error: false, refresh: () => {}, onOpen: () => {} },
  hasWorkflow: false,
  onWorkflow: () => {},
  onDiagnostics: () => {},
};

function Page() {
  return (
    <div
      data-preview-split=""
      data-summary-rail=""
      id="split"
      style={{ width: '1100px', height: '${PANE_HEIGHT}px', ['--summary-rail-width']: '${RAIL_WIDTH}px' }}
    >
      <div data-preview-area="column">
        <div data-preview-area="body">
          <div data-preview-area="header" style={{ height: '44px' }} />
          <div data-preview-area="transcript" data-preview-transcript="" style={{ minHeight: 0 }} />
        </div>
        <div data-preview-area="composer" style={{ height: '96px' }} />
      </div>
      <ChatSummaryRail id="rail" data-preview-area="rail" still state="open" onExited={() => {}} {...summary} />
    </div>
  );
}

createRoot(document.getElementById('root')).render(<Page />);
`;

/** Every teardown step gets this long and no longer (the recipe of `summaryPopover.browser.test.tsx`). */
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

interface Measured {
  card: { left: number; right: number; width: number; top: number; bottom: number };
  footerBottom: number;
  body: { clientHeight: number; scrollHeight: number; clientWidth: number; scrollWidth: number };
  /** Heights of the rows of each layout, and of every disclosure and Show all row. */
  dense: number[];
  comfortable: number[];
  toggles: number[];
  /** The rightmost edge any row, label or count reaches. */
  rightmost: number;
  pageScrollWidth: number;
  viewport: number;
  /** Names whose text is longer than their box: the instrument, proof the names could overflow. */
  cutNames: number;
  /** Every setup count (glyph and number) on a second line, against that line's box. */
  counts: {
    kind: string | null;
    top: number;
    height: number;
    left: number;
    right: number;
    glyph: { width: number; height: number; top: number; bottom: number };
    line: { top: number; height: number; left: number; right: number };
  }[];
  /** Each second line's parts in order: whether it may give way, its edges, whether it is cut. */
  lines: {
    left: number;
    right: number;
    /** The line's last part: the run time, or the quiet note in its place. */
    time: string | null;
    parts: { fit: boolean; left: number; right: number; width: number; cut: boolean }[];
  }[];
}

describe('the subagents in the summary card, in a real layout engine', () => {
  let browser: Browser | null = null;
  let server: BrowserServer | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    const launched = await launchChromium();
    browser = launched?.browser ?? null;
    server = launched?.server ?? null;
    if (!browser) {
      console.warn('No Chromium found: skipping the summary subagent tests.');
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

  async function mount(params: {
    agents: number;
    plan?: boolean;
    model?: string;
    gate?: boolean;
    markdownResult?: boolean;
  }): Promise<{
    tab: Page;
    measured: Measured;
  }> {
    const tab = await browser!.newPage({ viewport: { width: 1100, height: PANE_HEIGHT } });
    tab.setDefaultTimeout(8000);
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.evaluate((value) => {
      (window as unknown as { PARAMS: unknown }).PARAMS = value;
    }, params);
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('.br-agents');
    await tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );
    const measured = await tab.evaluate(() => {
      const card = document.querySelector('.br-summary-card')!.getBoundingClientRect();
      const body = document.querySelector<HTMLElement>('.br-summary__body')!;
      const heights = (selector: string) =>
        Array.from(document.querySelectorAll(selector)).map(
          (element) => element.getBoundingClientRect().height
        );
      const edges = Array.from(
        document.querySelectorAll(
          '.br-agents__row, .br-agents__phase-toggle, .br-agents__group-toggle, .br-agents__more, ' +
            '.br-agents__name, .br-agents__meta, .br-agents__trailing, .br-agents__phase-title, ' +
            '.br-agents__phase-counts, .br-agents__plan-title, .br-summary__count'
        )
      ).map((element) => element.getBoundingClientRect().right);
      return {
        card: {
          left: card.left,
          right: card.right,
          width: card.width,
          top: card.top,
          bottom: card.bottom,
        },
        footerBottom: document.querySelector('.br-summary__footer')!.getBoundingClientRect().bottom,
        body: {
          clientHeight: body.clientHeight,
          scrollHeight: body.scrollHeight,
          clientWidth: body.clientWidth,
          scrollWidth: body.scrollWidth,
        },
        dense: heights(".br-agents__row[data-layout='dense']"),
        comfortable: heights(".br-agents__row[data-layout='comfortable']"),
        toggles: heights('.br-agents__group-toggle, .br-agents__more, .br-agents__phase-toggle'),
        rightmost: Math.max(...edges),
        pageScrollWidth: document.documentElement.scrollWidth,
        viewport: window.innerWidth,
        cutNames: Array.from(document.querySelectorAll<HTMLElement>('.br-agents__name')).filter(
          (name) => name.scrollWidth > name.clientWidth
        ).length,
        counts: Array.from(document.querySelectorAll('.br-agents__meta .br-agents__count')).map(
          (count) => {
            const box = count.getBoundingClientRect();
            const glyph = count.querySelector('svg')!.getBoundingClientRect();
            const line = count.closest('.br-agents__meta')!.getBoundingClientRect();
            return {
              kind: count.getAttribute('data-count'),
              top: box.top,
              height: box.height,
              left: box.left,
              right: box.right,
              glyph: {
                width: glyph.width,
                height: glyph.height,
                top: glyph.top,
                bottom: glyph.bottom,
              },
              line: { top: line.top, height: line.height, left: line.left, right: line.right },
            };
          }
        ),
        lines: Array.from(document.querySelectorAll<HTMLElement>('.br-agents__meta')).map(
          (line) => {
            const box = line.getBoundingClientRect();
            return {
              left: box.left,
              right: box.right,
              time: line.lastElementChild?.textContent ?? null,
              parts: Array.from(line.querySelectorAll<HTMLElement>(':scope > *')).map((part) => {
                const edge = part.getBoundingClientRect();
                return {
                  fit: part.classList.contains('br-agents__fit'),
                  left: edge.left,
                  right: edge.right,
                  width: edge.width,
                  cut: part.scrollWidth > part.clientWidth,
                };
              }),
            };
          }
        ),
      };
    });
    return { tab, measured };
  }

  it('lays 30 subagents out as 28px rows that scroll inside the card at 240px', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, measured } = await mount({ agents: 30 });
    try {
      // The narrowest real card: the 240px column less the cell's 12px right padding.
      expect(measured.card.width).toBe(RAIL_WIDTH - 12);
      expect(measured.dense.length).toBeGreaterThan(12);
      for (const height of measured.dense) expect(height).toBe(28);
      for (const height of measured.toggles) expect(height).toBe(28);
      expect(measured.comfortable).toEqual([]);
      // The list scrolls inside the card's body; the card and its footer stay in the pane.
      expect(measured.body.scrollHeight).toBeGreaterThan(measured.body.clientHeight);
      expect(measured.card.bottom).toBeLessThanOrEqual(PANE_HEIGHT);
      expect(measured.footerBottom).toBeLessThanOrEqual(measured.card.bottom);
      // Nothing pushes sideways: long names and models are cut, not overflowing. The names
      // are long enough to overflow (the instrument), so the check below can fail.
      expect(measured.cutNames).toBeGreaterThan(0);
      expect(measured.body.scrollWidth).toBeLessThanOrEqual(measured.body.clientWidth);
      expect(measured.rightmost).toBeLessThanOrEqual(measured.card.right);
      expect(measured.pageScrollWidth).toBeLessThanOrEqual(measured.viewport);
    } finally {
      await tab.close();
    }
  });

  it('gives a short list comfortable two-line rows, still inside the card', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, measured } = await mount({ agents: 3 });
    try {
      expect(measured.dense).toEqual([]);
      expect(measured.comfortable).toHaveLength(3);
      // An 18px name line over a 16px setup line, inside 5px of padding each side.
      for (const height of measured.comfortable) expect(height).toBe(44);
      expect(measured.body.scrollWidth).toBeLessThanOrEqual(measured.body.clientWidth);
      expect(measured.rightmost).toBeLessThanOrEqual(measured.card.right);
    } finally {
      await tab.close();
    }
  });

  it('draws one muted second line, and keeps the peek left of its row, one at a time', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, measured } = await mount({ agents: 3 });
    try {
      // An 18px name over one 16px line; no setup counts on the row (F-2).
      for (const height of measured.comfortable) expect(height).toBe(44);
      expect(measured.counts).toEqual([]);
      expect(measured.lines).toHaveLength(3);
      for (const line of measured.lines)
        expect(line.right).toBeLessThanOrEqual(measured.card.right);

      const peekOf = (id: string) =>
        tab.evaluate(async (child) => {
          const row = document.querySelector(`[data-testid="subagent-row-${child}"]`)!;
          // The row's fill eases in over --dur-fast-min (95ms), so a read taken
          // as the peek mounts sees the transition's first frame, still
          // transparent. Measure the settled row.
          await Promise.all(row.getAnimations().map((animation) => animation.finished));
          const card = document.querySelector(`[data-testid="subagent-card-${child}"]`);
          const box = row.getBoundingClientRect();
          const peek = card?.getBoundingClientRect();
          return {
            open: document.querySelectorAll('[data-testid^="subagent-card-"]').length,
            pinned: row.getAttribute('data-pinned'),
            fill: getComputedStyle(row).backgroundColor,
            rowLeft: box.left,
            rowTop: box.top,
            peekRight: peek?.right ?? null,
            peekTop: peek?.top ?? null,
          };
        }, id);

      const resting = await peekOf('child-1');
      await tab.click('[data-testid="subagent-row-child-1"]');
      await tab.waitForSelector('[data-testid="subagent-card-child-1"]');
      // Off the row, so the fill measured is the kept one and not the hover's,
      // which is the same step.
      await tab.mouse.move(0, 0);
      const kept = await peekOf('child-1');
      expect(kept.open).toBe(1);
      expect(kept.pinned).toBe('true');
      // The kept row wears the strong fill; at rest it had none.
      expect(kept.fill).not.toBe(resting.fill);
      // The peek hangs to the LEFT of the row, 12px off, from its top.
      expect(kept.peekRight!).toBeLessThanOrEqual(kept.rowLeft);
      expect(Math.abs(kept.peekTop! - kept.rowTop)).toBeLessThanOrEqual(1);

      // Keeping another row's peek closes the first: one open at a time.
      await tab.click('[data-testid="subagent-row-child-2"]');
      await tab.waitForSelector('[data-testid="subagent-card-child-2"]');
      const second = await peekOf('child-2');
      expect(second.open).toBe(1);
      expect((await peekOf('child-1')).pinned).toBe(null);
    } finally {
      await tab.close();
    }
  });

  it('keeps a phase plan of 30 subagents to the same rows and the same width', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, measured } = await mount({ agents: 30, plan: true });
    try {
      for (const height of measured.dense) expect(height).toBe(28);
      expect(measured.body.scrollHeight).toBeGreaterThan(measured.body.clientHeight);
      expect(measured.body.scrollWidth).toBeLessThanOrEqual(measured.body.clientWidth);
      expect(measured.rightmost).toBeLessThanOrEqual(measured.card.right);
      expect(measured.pageScrollWidth).toBeLessThanOrEqual(measured.viewport);
    } finally {
      await tab.close();
    }
  });

  // R2-5: beside a gate chip, "1 of 1" broke into "1" / "of" / "1" at the rail's width, and a
  // result that began with a markdown table drew its pipes.
  it('keeps a gated phase to one line and a result line to plain words', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab } = await mount({ agents: 6, plan: true, gate: true, markdownResult: true });
    try {
      const phase = await tab.evaluate(() => {
        const toggle = document.querySelector<HTMLElement>(
          "[data-phase-id='plan'] .br-agents__phase-toggle"
        )!;
        const count = toggle.querySelector<HTMLElement>('.br-summary__count')!;
        const range = document.createRange();
        range.selectNodeContents(count);
        const lines = new Set(
          [...range.getClientRects()].filter((r) => r.width > 0).map((r) => Math.round(r.top))
        ).size;
        return {
          gate: Boolean(document.querySelector("[data-testid='phase-gate-plan']")),
          height: toggle.getBoundingClientRect().height,
          text: count.textContent,
          lines,
        };
      });
      expect(phase.gate, 'the instrument: the phase has its gate chip').toBe(true);
      expect(phase.text).toBe('4 of 4');
      expect(phase.lines).toBe(1);
      expect(phase.height).toBe(28);
    } finally {
      await tab.close();
    }
    // Four rows are comfortable, and the blocked one (the fourth) ended on a
    // result line that is a table's header row: no summary, so its status word
    // (live QA 2026-11-11: "Abstract · Flaw type(s) · Evidence · Bri…").
    const comfortable = await mount({ agents: 4, markdownResult: true });
    try {
      const lines = await comfortable.tab.evaluate(() =>
        [...document.querySelectorAll<HTMLElement>('.br-agents__meta')].map((m) => m.textContent)
      );
      expect(lines.join(' // ')).toContain('Needs an answer');
      expect(lines.join(' // ')).not.toContain('Patient');
      expect(lines.some((line) => line?.includes('|'))).toBe(false);
    } finally {
      await comfortable.tab.close();
    }
  });
});
