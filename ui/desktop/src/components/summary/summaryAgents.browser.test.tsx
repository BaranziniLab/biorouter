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
  resultLine: null,
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

  async function mount(params: { agents: number; plan?: boolean; model?: string }): Promise<{
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

  it('draws the setup counts as 12px glyphs on the 16px second line, and in full', async (ctx) => {
    if (!browser) return ctx.skip();
    // The fixture's model name is long enough to fill the line on its own (the instrument),
    // and a usual one is not.
    for (const model of [undefined, 'gpt-5.5']) {
      const { tab, measured } = await mount({ agents: 3, model });
      try {
        for (const height of measured.comfortable) expect(height).toBe(44);
        // Extensions, skills and knowledge bases on each of the three rows.
        expect(measured.counts.map((count) => count.kind)).toEqual(
          Array.from({ length: 3 }, () => ['extension', 'skill', 'knowledge']).flat()
        );
        for (const count of measured.counts) {
          // Held to the 16px line, so the row does not grow.
          expect(count.line.height).toBe(16);
          expect(count.height).toBe(16);
          expect(count.top).toBe(count.line.top);
          expect(count.glyph.width).toBe(12);
          expect(count.glyph.height).toBe(12);
          expect(count.glyph.top - count.top).toBe(2);
          // Drawn in full: the line's ellipsis never takes a count.
          expect(count.left).toBeGreaterThanOrEqual(count.line.left);
          expect(count.right).toBeLessThanOrEqual(count.line.right);
        }
        expect(measured.lines).toHaveLength(3);
        for (const line of measured.lines) {
          // Model, dot, counts, dot, and the run time or (on the fixture's first row, quiet
          // for 10 minutes) the quiet note in its place.
          expect(line.parts.map((part) => part.fit)).toEqual([true, false, false, false, false]);
          const [modelPart, ...rest] = line.parts;
          const time = rest[rest.length - 1];
          for (const part of line.parts) expect(part.right).toBeLessThanOrEqual(line.right);
          // The dots and the counts never give way; the time only once the model is gone.
          for (const part of rest.slice(0, -1)) expect(part.cut).toBe(false);
          if (time.cut) expect(modelPart.width).toBe(0);
        }
        expect(measured.lines.map((line) => line.time)).toEqual([
          'quiet for 10m',
          '1h 5m',
          '1h 5m',
        ]);
        // Beside a run time the counts leave room: the long model name is cut and a usual one
        // is not, and the time is whole.
        for (const line of measured.lines.slice(1)) {
          expect(line.parts[0].cut).toBe(!model);
          expect(line.parts[0].width).toBeGreaterThan(0);
          expect(line.parts[4].cut).toBe(false);
        }
        // The narrowest case, a private row (its lock takes 20px of the line) that is also quiet:
        // the model gives all its room before the note's tail goes.
        expect(measured.lines[0].parts[0].width).toBe(0);
        expect(measured.body.scrollWidth).toBeLessThanOrEqual(measured.body.clientWidth);
        expect(measured.rightmost).toBeLessThanOrEqual(measured.card.right);
      } finally {
        await tab.close();
      }
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
});
