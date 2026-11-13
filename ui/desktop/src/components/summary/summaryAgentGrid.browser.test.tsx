// @vitest-environment node
/**
 * The agent grid in the Chat summary, measured in Chromium with the whole stylesheet.
 *
 * jsdom lays nothing out, so `agentGrid.test.ts` and `SummarySubagents.test.tsx` can say which
 * marks and rows exist and never how big a square is, whether 150 of them wrap inside the card,
 * or where a square's card hangs. This file bundles the real `ChatSummaryRail` with esbuild (the
 * recipe of `summaryAgents.browser.test.tsx`), mounts it in the rail's own grid at its narrowest
 * (a 240px column, a 228px card), and measures the squares at 1, 8 and 150 agents, a hover on a
 * square, and the shapes the statuses draw.
 *
 * It also checks the owner's rule of 2026-11-12 in the real cascade: the sections are parted by
 * one hairline across the card, a phase by an inset one, and nothing between a square and the card
 * has a fill or a radius, so the grid sits on the card's own ground.
 *
 * With `SUMMARY_GRID_SHOTS=<dir>` it also saves PNGs of the rail for review, each in light and dark:
 * three agents (`agents-3`), twenty agents across a plan with gates, a decision-model subagent and
 * monitors (`phases-20`), 150 agents across three phases with decision-model subagents among them
 * (`agents-150`), and a square's card (`peek`).
 */
import { mkdirSync, readFileSync } from 'node:fs';
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
const SHEETS = [
  'chatSummary.css',
  'summaryRail.css',
  'summaryAgents.css',
  'summaryAgentGrid.css',
  'summaryDecisions.css',
  '../bottom_menu/pickers.css',
]
  .map((name) => readFileSync(resolve(here, name), 'utf8'))
  .join('\n');
const SHOTS = process.env.SUMMARY_GRID_SHOTS ?? '';

const RAIL_WIDTH = 240;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ChatSummaryRail } from './ChatSummaryRail';
import { parseSessionSubagents } from '../../utils/sessionSubagents';
import { parseDecisionSettings, parseSessionDecisionSummary } from '../../utils/sessionDecisions';

const params = window.PARAMS;
const ROLES = ['Reader', 'Auditor', 'Tabulator', 'Checker', 'Summariser', 'Critic'];
const MODELS = [
  ['anthropic', 'claude-sonnet-4-5-20250929'],
  ['openai', 'gpt-5.5'],
  ['anthropic', 'claude-opus-4-5-20251101'],
];

function row(i, status, extra) {
  const [providerName, modelName] = MODELS[i % MODELS.length];
  return Object.assign({
    sessionId: 'child-' + i,
    name: 'Read the trial reports in batch ' + (i + 1) + ' and tabulate the dosing schedules',
    role: ROLES[i % ROLES.length] + ' ' + (i + 1),
    phaseId: null,
    status,
    loopPhase: null,
    quietSeconds: 3,
    createdAt: '2026-11-04T10:00:' + String(i % 60).padStart(2, '0') + 'Z',
    updatedAt: '2026-11-04T10:' + String(10 + (i % 40)).padStart(2, '0') + ':00Z',
    endedAt: status === 'completed' || status === 'failed' ? '2026-11-04T10:' + String(10 + (i % 40)).padStart(2, '0') + ':00Z' : null,
    elapsedSeconds: 40 + i * 13,
    providerName,
    modelName,
    extensions: ['developer', 'webdocuments'],
    skills: ['scientific-research'],
    knowledgeBases: [],
    privacyTier: 'public',
    totalTokens: 12000 + i * 100,
    messageCount: 9,
    toolCallCount: 4,
    resultLine: status === 'completed' ? 'Tabulated 14 dosing schedules from 6 reports' : status === 'failed' ? 'The registry export could not be read' : null,
  }, extra || {});
}

const STATUS_CYCLE = ['completed', 'completed', 'running', 'completed', 'failed', 'completed', 'queued', 'blocked', 'completed', 'running'];
/** A subagent whose own model is a decision model: decision-model work, drawn as a circle. */
const JUDGE = { role: 'Dose check', providerName: 'ollama_decision', modelName: 'tev1:0.8b' };
const counts = (list) => ({
  total: list.length,
  done: list.filter((r) => r.status === 'completed').length,
  running: list.filter((r) => r.status === 'running' || r.status === 'queued').length,
  blocked: list.filter((r) => r.status === 'blocked').length,
  failed: list.filter((r) => r.status === 'failed').length,
  quiet: list.filter((r) => r.quietSeconds > 180 && r.status === 'running').length,
});
let rows = [];
let plan = null;
if (params.scenario === 'three') {
  rows = [
    row(0, 'running'),
    row(1, 'completed'),
    row(2, 'completed', { role: 'Dose check', providerName: 'ollama_decision', modelName: 'tev1:0.8b', resultLine: 'Yes: every dose is within the label range' }),
  ];
} else if (params.scenario === 'phases') {
  const audit = Array.from({ length: 8 }, (_, i) => row(i, i === 5 ? 'failed' : 'completed', { phaseId: 'audit' }));
  const draft = Array.from({ length: 12 }, (_, i) => {
    const status = ['running', 'running', 'completed', 'running', 'queued', 'completed', 'blocked', 'running', 'queued', 'completed', 'running', 'queued'][i];
    // One of the drafters' checks runs on a decision model: a circle among the squares.
    const judge = i === 10 ? JUDGE : {};
    return row(8 + i, status, Object.assign({ phaseId: 'draft', quietSeconds: i === 3 ? 600 : 3 }, judge));
  });
  rows = [...audit, ...draft];
  plan = {
    title: 'Survey every trial of the drug',
    goal: null,
    revision: 3,
    phases: [
      { id: 'audit', title: 'Audit the sources', kind: 'auditing', status: 'completed', dependsOn: [],
        agentIds: audit.map((r) => r.sessionId),
        counts: { total: 8, done: 8, running: 0, blocked: 0, failed: 1, quiet: 0 },
        gate: { threshold: 0.8, questionCount: 2, attempts: 1, maxAttempts: 3, lastPassed: true,
          lastMinP: 0.91, questions: { cited: 'Is every source cited?' } },
        handoff: null, startedAt: null, finishedAt: null, heldBy: null },
      { id: 'draft', title: 'Draft the dosing tables', kind: 'execution', status: 'in_progress', dependsOn: ['audit'],
        agentIds: draft.map((r) => r.sessionId),
        counts: { total: 12, done: 3, running: 5, blocked: 1, failed: 0, quiet: 1 },
        gate: { threshold: 0.8, questionCount: 1, attempts: 0, maxAttempts: 3, lastPassed: null,
          lastMinP: null, questions: { complete: 'Are the tables complete?' } },
        handoff: null, startedAt: null, finishedAt: null, heldBy: null },
      { id: 'check', title: 'Check the numbers', kind: 'verification', status: 'pending', dependsOn: ['draft'],
        agentIds: [], counts: { total: 0, done: 0, running: 0, blocked: 0, failed: 0, quiet: 0 },
        gate: null, handoff: null, startedAt: null, finishedAt: null, heldBy: 'draft' },
    ],
  };
} else if (params.scenario === 'plan150') {
  // 150 agents in three phases: a finished audit with two failures, a verification in
  // full swing (every status, one gone quiet), and a report waiting to start. Some of
  // each phase's checks run on a decision model.
  const audit = Array.from({ length: 40 }, (_, i) =>
    row(i, i === 13 || i === 29 ? 'failed' : 'completed', Object.assign({ phaseId: 'audit' }, i % 8 === 3 ? JUDGE : {}))
  );
  const VERIFY = ['completed', 'running', 'completed', 'queued', 'completed', 'running', 'failed', 'completed', 'blocked', 'completed', 'running', 'queued'];
  const verify = Array.from({ length: 90 }, (_, i) =>
    row(40 + i, VERIFY[i % VERIFY.length], Object.assign(
      { phaseId: 'verify', quietSeconds: i === 13 ? 900 : 3 },
      i % 7 === 2 ? JUDGE : {}
    ))
  );
  const report = Array.from({ length: 20 }, (_, i) => row(130 + i, 'queued', { phaseId: 'report' }));
  rows = [...audit, ...verify, ...report];
  plan = {
    title: 'Review every trial of the drug',
    goal: null,
    revision: 5,
    phases: [
      { id: 'audit', title: 'Audit the sources', kind: 'auditing', status: 'completed', dependsOn: [],
        agentIds: audit.map((r) => r.sessionId), counts: counts(audit),
        gate: { threshold: 0.8, questionCount: 2, attempts: 1, maxAttempts: 3, lastPassed: true,
          lastMinP: 0.93, questions: { cited: 'Is every source cited?' } },
        handoff: null, startedAt: null, finishedAt: null, heldBy: null },
      { id: 'verify', title: 'Verify each claim', kind: 'verification', status: 'in_progress', dependsOn: ['audit'],
        agentIds: verify.map((r) => r.sessionId), counts: counts(verify),
        gate: { threshold: 0.8, questionCount: 1, attempts: 0, maxAttempts: 3, lastPassed: null,
          lastMinP: null, questions: { checked: 'Is every claim checked?' } },
        handoff: null, startedAt: null, finishedAt: null, heldBy: null },
      { id: 'report', title: 'Write the report', kind: 'synthesis', status: 'pending', dependsOn: ['verify'],
        agentIds: report.map((r) => r.sessionId), counts: counts(report),
        gate: null, handoff: null, startedAt: null, finishedAt: null, heldBy: 'verify' },
    ],
  };
} else {
  rows = Array.from({ length: params.agents }, (_, i) => row(i, STATUS_CYCLE[i % STATUS_CYCLE.length]));
}
const data = parseSessionSubagents({ parentSessionId: 'parent', subagents: rows, plan });
const decisions = params.decisions
  ? {
      sessionId: 'parent',
      data: {
        summary: parseSessionDecisionSummary({
          total: 212,
          aggregates: [
            { origin: 'monitor', originId: 'scope', label: 'Scope drift', count: 40, passed: 0, failed: 40, errors: 0,
              lastAt: '2026-11-04T11:00:00Z', lastAnswer: 'yes', lastPassProbability: 0.9 },
            { origin: 'monitor', originId: 'cost', label: 'Cost check', count: 38, passed: 0, failed: 38, errors: 0,
              lastAt: '2026-11-04T11:00:00Z', lastAnswer: 'no', lastPassProbability: 0.1 },
            { origin: 'tool', originId: 'call-1', label: 'decide', count: 134, passed: 90, failed: 44, errors: 0,
              lastAt: '2026-11-04T11:02:00Z', lastAnswer: 'yes', lastPassProbability: 0.8 },
          ],
          recentTurns: [],
        }),
        settings: parseDecisionSettings({
          defaultModel: null,
          monitors: [
            { id: 'scope', name: 'Scope drift', question: { instructions: 'Has the agent left the task?', kind: { binary: {} } },
              threshold: 0.8, action: 'notify', applyToSubagents: true },
            { id: 'cost', name: 'Cost check', question: { instructions: 'Is the spend over budget?', kind: { binary: {} } },
              threshold: 0.8, action: 'stop' },
          ],
        }),
      },
      loading: false, error: false, refresh: () => {},
      onOpenHistory: () => {}, onOpenMonitors: () => {},
      fired: new Set(['scope']),
    }
  : undefined;
const summary = {
  toolCalls: '42',
  billedTokens: '211k',
  artifacts: '3',
  codeDelta: { added: 120, removed: 14 },
  todos: {
    items: params.todos
      ? [
          { id: '1', text: 'Delegate the reading', status: 'completed' },
          { id: '2', text: 'Reconcile the tables', status: 'in_progress' },
          { id: '3', text: 'Write the summary', status: 'pending' },
        ]
      : [],
    loading: false, error: false, refresh: () => {},
  },
  subagents: { data, loading: false, error: false, refresh: () => {}, onOpen: () => {}, onOpenGate: () => {} },
  decisions,
  chatModel: 'gpt-5.5',
  privacyTier: 'public',
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
      style={{ width: '1100px', height: params.height + 'px', ['--summary-rail-width']: '${RAIL_WIDTH}px' }}
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

interface Params {
  scenario: 'count' | 'three' | 'phases' | 'plan150';
  agents?: number;
  todos?: boolean;
  decisions?: boolean;
  height?: number;
  dark?: boolean;
}

interface Clip {
  left: number;
  top: number;
  width: number;
  height: number;
}

describe('the agent grid in the summary card, in a real layout engine', () => {
  let browser: Browser | null = null;
  let server: BrowserServer | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    const launched = await launchChromium();
    browser = launched?.browser ?? null;
    server = launched?.server ?? null;
    if (!browser) {
      console.warn('No Chromium found: skipping the agent grid tests.');
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
    if (SHOTS) mkdirSync(SHOTS, { recursive: true });
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

  async function mount(params: Params): Promise<Page> {
    const height = params.height ?? 720;
    const tab = await browser!.newPage({
      viewport: { width: 1100, height },
      deviceScaleFactor: SHOTS ? 2 : 1,
      reducedMotion: 'reduce',
    });
    tab.setDefaultTimeout(8000);
    await tab.setContent(
      `<!doctype html><html class="${params.dark ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.evaluate(
      (value) => {
        (window as unknown as { PARAMS: unknown }).PARAMS = value;
      },
      { ...params, height }
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('.br-summary-card');
    await tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );
    return tab;
  }

  /** Every square and row mark's box, the card's, and what overflows. */
  const measure = (tab: Page) =>
    tab.evaluate(() => {
      const box = (element: Element) => {
        const rect = element.getBoundingClientRect();
        return {
          left: rect.left,
          right: rect.right,
          top: rect.top,
          bottom: rect.bottom,
          width: rect.width,
          height: rect.height,
        };
      };
      const body = document.querySelector<HTMLElement>('.br-summary__body')!;
      return {
        card: box(document.querySelector('.br-summary-card')!),
        cells: Array.from(document.querySelectorAll('.br-agent-grid__cell .br-agent-mark')).map(
          box
        ),
        rowMarks: Array.from(document.querySelectorAll('.br-agents__row .br-agent-mark')).map(box),
        rows: Array.from(document.querySelectorAll('.br-agents__row')).map(box),
        bodyOverflow: body.scrollWidth - body.clientWidth,
        pageOverflow: document.documentElement.scrollWidth - window.innerWidth,
      };
    });

  /**
   * Until a peek is placed: Radix draws a popover it has not placed yet at `translate(0, -200%)`
   * (left 0, above the page), and a box read then says nothing about where the card hangs. Under
   * load the read landed there, and an assertion that the card ends left of the square passed
   * against it whatever the placement. Then two frames, so a re-placement settles.
   */
  const placedPeek = async (tab: Page, testId: string) => {
    await tab.waitForFunction((id) => {
      const card = document.querySelector(`[data-testid="${id}"]`);
      const wrapper = card?.closest<HTMLElement>('[data-radix-popper-content-wrapper]');
      return (
        !!wrapper && wrapper.style.transform !== '' && !wrapper.style.transform.includes('-200%')
      );
    }, testId);
    await tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );
  };

  const shot = async (tab: Page, name: string, clip?: Clip) => {
    if (!SHOTS) return;
    if (clip) {
      await tab.screenshot({
        path: resolve(SHOTS, name),
        clip: { x: clip.left, y: clip.top, width: clip.width, height: clip.height },
      });
    } else {
      await tab.locator('.br-summary-card').screenshot({ path: resolve(SHOTS, name) });
    }
  };

  it('lists one agent as a row led by a 14px square, with no grid', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await mount({ scenario: 'count', agents: 1 });
    try {
      const measured = await measure(tab);
      expect(measured.cells).toEqual([]);
      expect(measured.rowMarks).toHaveLength(1);
      // A few agents read larger than a grid's 10px squares: the to-do list's 14px glyph.
      expect(measured.rowMarks[0].width).toBe(14);
      expect(measured.rowMarks[0].height).toBe(14);
      // The comfortable row: an 18px name line over a 16px second line.
      expect(measured.rows[0].height).toBe(44);
    } finally {
      await tab.close();
    }
  });

  it('draws eight agents as a grid of 10px squares, then five rows', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await mount({ scenario: 'count', agents: 8 });
    try {
      const measured = await measure(tab);
      expect(measured.cells).toHaveLength(8);
      for (const cell of measured.cells) {
        expect(cell.width).toBe(10);
        expect(cell.height).toBe(10);
      }
      // One line of squares, 4px apart.
      expect(new Set(measured.cells.map((cell) => cell.top)).size).toBe(1);
      expect(measured.cells[1].left - measured.cells[0].right).toBe(4);
      // The head: five 28px rows, each led by the grid's own 10px square.
      expect(measured.rows).toHaveLength(5);
      for (const row of measured.rows) expect(row.height).toBe(28);
      for (const mark of measured.rowMarks) expect(mark.width).toBe(10);
      expect(measured.bodyOverflow).toBeLessThanOrEqual(0);
    } finally {
      await tab.close();
    }
  });

  it('packs 150 agents into a dense block of 7px squares inside the card', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await mount({ scenario: 'count', agents: 150, height: 900 });
    try {
      const measured = await measure(tab);
      expect(measured.cells).toHaveLength(150);
      for (const cell of measured.cells) {
        expect(cell.width).toBe(7);
        expect(cell.height).toBe(7);
      }
      // They wrap in rows 2px apart, every one inside the card, nothing sideways.
      const lines = [...new Set(measured.cells.map((cell) => cell.top))].sort((a, b) => a - b);
      expect(lines.length).toBeGreaterThan(4);
      expect(lines[1] - lines[0]).toBe(9);
      for (const cell of measured.cells) {
        expect(cell.left).toBeGreaterThanOrEqual(measured.card.left);
        expect(cell.right).toBeLessThanOrEqual(measured.card.right);
      }
      expect(measured.rows).toHaveLength(5);
      expect(measured.bodyOverflow).toBeLessThanOrEqual(0);
      expect(measured.pageOverflow).toBeLessThanOrEqual(0);
    } finally {
      await tab.close();
    }
  });

  it('peeks a square’s card on hover, to the left of the rail, and keeps it on click', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await mount({ scenario: 'phases', todos: true, decisions: true, height: 1000 });
    try {
      const cell = tab.locator('[data-testid="subagent-cell-child-9"]');
      await cell.hover();
      const card = tab.locator('[data-testid="subagent-card-child-9"]');
      await card.waitFor();
      await placedPeek(tab, 'subagent-card-child-9');
      const placed = await tab.evaluate(() => {
        const cardBox = document
          .querySelector('[data-testid="subagent-card-child-9"]')!
          .getBoundingClientRect();
        const rail = document.querySelector('.br-summary-card')!.getBoundingClientRect();
        const square = document
          .querySelector('[data-testid="subagent-cell-child-9"]')!
          .getBoundingClientRect();
        return {
          right: cardBox.right,
          railLeft: rail.left,
          squareLeft: square.left,
          pinned: document
            .querySelector('[data-testid="subagent-cell-child-9"]')!
            .getAttribute('data-pinned'),
          text: document.querySelector('[data-testid="subagent-card-child-9"]')!.textContent,
        };
      });
      expect(placed.right).toBeLessThanOrEqual(placed.squareLeft);
      // It hangs clear of the rail, 8px off the card's own edge, so it never covers the rows'
      // marks (review, 2026-11-12: from a square mid-grid it lay over the glyph column).
      expect(Math.abs(placed.railLeft - 8 - placed.right)).toBeLessThanOrEqual(1);
      // A hover is a preview: the square stays at rest.
      expect(placed.pinned).toBe(null);
      // The card names the agent, its model and where it runs, its status and time, and Open.
      expect(placed.text).toContain('Checker 10');
      expect(placed.text).toContain('claude-sonnet-4-5 · Public');
      expect(placed.text).toContain('Running');
      expect(placed.text).toContain('Open in a tab');
      // The chat's monitor that watches its subagents is named on the card.
      expect(placed.text).toContain('Watched by Scope drift');
      await cell.click();
      await tab.mouse.move(0, 0);
      expect(await cell.getAttribute('data-pinned')).toBe('true');
    } finally {
      await tab.close();
    }
  });

  it('draws each status by its own shape, and decision work as circles', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await mount({ scenario: 'phases', todos: true, decisions: true, height: 1000 });
    try {
      const marks = await tab.evaluate(() =>
        Array.from(document.querySelectorAll<SVGElement>('.br-agent-mark')).map((mark) => ({
          kind: mark.getAttribute('data-kind'),
          shape: mark.getAttribute('data-shape'),
          parts: Array.from(mark.children).map(
            (child) =>
              `${child.tagName}.${child.getAttribute('class')}.${child.getAttribute('fill-rule') ?? ''}`
          ),
          color: getComputedStyle(mark).color,
        }))
      );
      const byKind = new Map(marks.map((mark) => [mark.kind, mark]));
      // Every kind the scenario has is drawn by a different set of parts.
      const signatures = new Set(
        ['queued', 'running', 'waiting', 'done', 'failed'].map((kind) =>
          byKind.get(kind)!.parts.join('+')
        )
      );
      expect(signatures.size).toBe(5);
      // And by a different ink: running, waiting, failed and done are four colours.
      const inks = new Set(
        ['running', 'waiting', 'failed', 'done'].map((kind) => byKind.get(kind)!.color)
      );
      expect(inks.size).toBe(4);
      // The gates and the Decisions rows are circles; every subagent here is a square.
      const gates = await tab.$$eval('.br-agents__gate .br-agent-mark', (found) =>
        found.map((mark) => mark.getAttribute('data-shape'))
      );
      expect(gates).toEqual(['circle', 'circle']);
      const decisions = await tab.$$eval('.br-decisions .br-agent-mark', (found) =>
        found.map((mark) => mark.getAttribute('data-shape'))
      );
      expect(decisions.length).toBeGreaterThan(0);
      expect(new Set(decisions)).toEqual(new Set(['circle']));
    } finally {
      await tab.close();
    }
  });

  /**
   * The owner's rule (2026-11-12): sections, never boxes. Every section after the first draws
   * one hairline across the card; a phase after the first draws one inset to the content; and
   * every element between a square and the card is transparent and square-cornered, so the grid
   * sits on the card's own ground. Only the peek floats on a surface of its own.
   */
  it('parts sections and phases by hairlines and draws the grid on the card’s own ground', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await mount({ scenario: 'phases', todos: true, decisions: true, height: 1000 });
    try {
      const found = await tab.evaluate(() => {
        const card = document.querySelector('.br-summary-card')!;
        const cardBox = card.getBoundingClientRect();
        const cardFill = getComputedStyle(card).backgroundColor;
        const edge = (element: Element) => {
          const style = getComputedStyle(element);
          const box = element.getBoundingClientRect();
          return {
            width: style.borderTopWidth,
            style: style.borderTopStyle,
            left: box.left - cardBox.left,
            right: cardBox.right - box.right,
          };
        };
        const sections = Array.from(document.querySelectorAll('.br-summary__body > section')).map(
          (section) => ({ label: section.getAttribute('aria-label'), ...edge(section) })
        );
        const phases = Array.from(document.querySelectorAll('.br-agents__phase')).map(edge);
        // From each square up to the card: no fill, no radius, no line but the dividers.
        const boxed: string[] = [];
        for (const cell of Array.from(document.querySelectorAll('.br-agent-grid__cell'))) {
          for (let node = cell.parentElement; node && node !== card; node = node.parentElement) {
            const style = getComputedStyle(node);
            const filled = style.backgroundColor !== 'rgba(0, 0, 0, 0)';
            const rounded = style.borderTopLeftRadius !== '0px';
            const outlined = ['Left', 'Right', 'Bottom'].some(
              (side) => style.getPropertyValue(`border-${side.toLowerCase()}-width`) !== '0px'
            );
            if (filled || rounded || outlined) {
              boxed.push(`${node.tagName}.${node.className}`);
            }
          }
        }
        // The gate's circle and its word rest on the card's ground too.
        const gates = Array.from(document.querySelectorAll('.br-agents__gate')).map(
          (gate) => getComputedStyle(gate).backgroundColor
        );
        return { cardFill, sections, phases, boxed: [...new Set(boxed)], gates };
      });
      expect(found.boxed).toEqual([]);
      for (const ground of found.gates) expect(ground).toBe('rgba(0, 0, 0, 0)');
      // To do, Plan, Decisions, This chat: the first draws no line, the rest one hairline each,
      // across the card's whole width inside its 1px transparent border.
      expect(found.sections.map((section) => section.label)).toEqual([
        'To do',
        'Plan: Survey every trial of the drug',
        'Decisions',
        'This chat',
      ]);
      expect(found.sections[0].width).toBe('0px');
      for (const section of found.sections.slice(1)) {
        expect(section.width).toBe('1px');
        expect(section.style).toBe('solid');
        expect(section.left).toBe(1);
        expect(section.right).toBe(1);
      }
      // A phase's hairline is inset to the content (the body's 12px gutter), so it never reads
      // as a section's.
      expect(found.phases[0].width).toBe('0px');
      for (const phase of found.phases.slice(1)) {
        expect(phase.width).toBe('1px');
        expect(phase.left).toBe(13);
        expect(phase.right).toBe(13);
      }
      // A decision-model subagent is a circle among the squares, and the key says which is which,
      // on one line under the Plan heading and above the first phase it explains.
      const key = await tab.locator('[data-testid="summary-agents-key"]').textContent();
      expect(key).toBe('Models:languagedecision');
      const placedKey = await tab.evaluate(() => {
        const box = (selector: string) => document.querySelector(selector)!.getBoundingClientRect();
        const key = box('[data-testid="summary-agents-key"]');
        const heading = box('[data-testid="summary-plan-toggle"]');
        const phase = box('.br-agents__phase');
        const items = Array.from(document.querySelectorAll('.br-agents__key-item')).map(
          (item) => item.getBoundingClientRect().top
        );
        return { key, heading, phase, lines: new Set(items).size };
      });
      expect(placedKey.key.top).toBeGreaterThanOrEqual(placedKey.heading.bottom - 1);
      expect(placedKey.key.bottom).toBeLessThanOrEqual(placedKey.phase.top);
      expect(placedKey.lines).toBe(1);
      // A phase with no subagents and no gate ends its line with its status word on the rows'
      // right edge, beside a title on one line (review, 2026-11-12: a reserved "Add gate" slot
      // left "Pending" adrift mid-line and wrapped the title).
      const pending = await tab.evaluate(() => {
        const card = document.querySelector('.br-summary-card')!.getBoundingClientRect();
        const line = document.querySelector('[data-phase-id="check"] .br-agents__phase-line')!;
        const word = line.querySelector('.br-summary__count')!.getBoundingClientRect();
        const title = line.querySelector('.br-agents__phase-title')!.getBoundingClientRect();
        return {
          edge: card.right - 13,
          wordRight: word.right,
          wordTop: word.top,
          titleTop: title.top,
          titleHeight: title.height,
          text: line.querySelector('.br-summary__count')!.textContent,
        };
      });
      expect(pending.text).toBe('Pending');
      expect(Math.abs(pending.wordRight - pending.edge)).toBeLessThanOrEqual(1);
      expect(Math.abs(pending.wordTop - pending.titleTop)).toBeLessThanOrEqual(2);
      expect(pending.titleHeight).toBeLessThanOrEqual(18);
      expect(
        await tab.getAttribute(
          '[data-testid="subagent-cell-child-18"] .br-agent-mark',
          'data-shape'
        )
      ).toBe('circle');
    } finally {
      await tab.close();
    }
  });

  it('saves the review screenshots, in light and dark', async (ctx) => {
    if (!browser || !SHOTS) return ctx.skip();
    for (const dark of [false, true]) {
      const mode = dark ? 'dark' : 'light';
      const three = await mount({ scenario: 'three', todos: true, height: 760, dark });
      try {
        await three.click('[data-testid="summary-details-toggle"]');
        await three.mouse.move(0, 0);
        await shot(three, `agents-3-${mode}.png`);
      } finally {
        await three.close();
      }
      const phases = await mount({
        scenario: 'phases',
        todos: true,
        decisions: true,
        height: 1100,
        dark,
      });
      try {
        await shot(phases, `phases-20-${mode}.png`);
        const cell = phases.locator('[data-testid="subagent-cell-child-9"]');
        await cell.hover();
        await phases.locator('[data-testid="subagent-card-child-9"]').waitFor();
        await placedPeek(phases, 'subagent-card-child-9');
        const box = await phases.evaluate(() => {
          const card = document
            .querySelector('[data-testid="subagent-card-child-9"]')!
            .getBoundingClientRect();
          const rail = document.querySelector('.br-summary-card')!.getBoundingClientRect();
          const top = Math.min(card.top, rail.top) - 8;
          return {
            left: card.left - 8,
            top,
            width: rail.right - card.left + 16,
            height: Math.max(card.bottom, rail.top + 520) - top + 8,
          };
        });
        await shot(phases, `peek-${mode}.png`, box);
      } finally {
        await phases.close();
      }
      const many = await mount({
        scenario: 'plan150',
        todos: true,
        decisions: true,
        height: 1500,
        dark,
      });
      try {
        const fits = await many.evaluate(() => {
          const body = document.querySelector<HTMLElement>('.br-summary__body')!;
          return body.scrollHeight <= body.clientHeight;
        });
        expect(fits, 'the 150-agent card fits its screenshot').toBe(true);
        await shot(many, `agents-150-${mode}.png`);
      } finally {
        await many.close();
      }
    }
  }, 180_000);
});
