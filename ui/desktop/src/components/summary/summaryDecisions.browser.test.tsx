// @vitest-environment node
/**
 * The chat summary's Decisions section, measured in Chromium with the whole stylesheet.
 *
 * jsdom lays nothing out, so `SummaryDecisions.test.tsx` can say which rows exist and never how
 * tall they are or whether a long monitor name pushes the card sideways. This file bundles the
 * real `ChatSummaryRail` with esbuild, mounts it in the rail's own grid at its narrowest, a 240px
 * column, with a chat that made 5,000 decision calls (four monitors, fifty decide calls, a gate),
 * compiles `styles/main.css` with Tailwind over the classes the bundle uses, adds the summary's
 * stylesheets unlayered as the app does, and measures.
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
const SHEETS = [
  resolve(here, 'chatSummary.css'),
  resolve(here, 'summaryRail.css'),
  resolve(here, 'summaryAgents.css'),
  resolve(here, 'summaryDecisions.css'),
  resolve(here, '../decisions/probabilityBar.css'),
]
  .map((path) => readFileSync(path, 'utf8'))
  .join('\n');

const RAIL_WIDTH = 240;
const PANE_HEIGHT = 720;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ChatSummaryRail } from './ChatSummaryRail';
import { parseDecisionSettings, parseSessionDecisionSummary } from '../../utils/sessionDecisions';

const monitors = [
  'Consent form reached and every required field on it filled in',
  'Asked for a password',
  'Stuck on the same page',
  'Task achieved',
].map((name, i) => ({
  id: 'm' + i,
  name,
  question: { instructions: name + '?', kind: { binary: {} } },
  threshold: 0.85,
  enabled: i !== 2,
}));
const aggregates = [
  ...monitors.map((m, i) => ({
    origin: 'monitor', originId: m.id, label: m.name, count: 25, passed: 0, failed: 25,
    errors: i === 3 ? 25 : 0, lastAt: '2026-11-04T11:00:00Z',
    lastAnswer: i === 3 ? null : 'no', lastPassProbability: i === 3 ? null : 0.31,
  })),
  ...Array.from({ length: 50 }, (_, i) => ({
    origin: 'tool', originId: 'call-' + i, label: 'decide', count: 96, passed: 40, failed: 56,
    errors: 0, lastAt: '2026-11-04T10:' + String(i).padStart(2, '0') + ':00Z',
    lastAnswer: 'yes', lastPassProbability: 0.93,
  })),
  { origin: 'gate', originId: 'verify', label: 'Check every number in the final tables against the source',
    count: 100, passed: 1, failed: 99, errors: 0, lastAt: '2026-11-04T11:05:00Z',
    lastAnswer: 'no', lastPassProbability: 0.62 },
];
const summary = parseSessionDecisionSummary({ total: 5000, aggregates, recentTurns: [] });
const settings = parseDecisionSettings({ defaultModel: null, monitors });

const props = {
  toolCalls: '42',
  billedTokens: '211k',
  artifacts: '3',
  codeDelta: { added: 120, removed: 14 },
  todos: { items: [], loading: false, error: false, refresh: () => {} },
  decisions: {
    sessionId: 's1',
    data: { summary, settings },
    loading: false,
    error: false,
    refresh: () => {},
    onOpenHistory: () => {},
    onOpenMonitors: () => {},
  },
  hasWorkflow: false,
  onWorkflow: () => {},
  onDiagnostics: () => {},
};

function Page() {
  return (
    <div
      data-preview-split=""
      data-summary-rail=""
      style={{ width: '1100px', height: '${PANE_HEIGHT}px', ['--summary-rail-width']: '${RAIL_WIDTH}px' }}
    >
      <div data-preview-area="column">
        <div data-preview-area="body">
          <div data-preview-area="header" style={{ height: '44px' }} />
          <div data-preview-area="transcript" data-preview-transcript="" style={{ minHeight: 0 }} />
        </div>
        <div data-preview-area="composer" style={{ height: '96px' }} />
      </div>
      <ChatSummaryRail id="rail" data-preview-area="rail" still state="open" onExited={() => {}} {...props} />
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

describe('the Decisions section in the summary card, in a real layout engine', () => {
  let browser: Browser | null = null;
  let server: BrowserServer | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    const launched = await launchChromium();
    browser = launched?.browser ?? null;
    server = launched?.server ?? null;
    if (!browser) {
      console.warn('No Chromium found: skipping the summary decisions tests.');
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

  async function mount(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1100, height: PANE_HEIGHT } });
    tab.setDefaultTimeout(8000);
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('.br-decisions');
    await tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );
    return tab;
  }

  it('draws one row per source at 240px: 28px or taller, inside the card, no sideways scroll', async () => {
    if (!browser) return;
    const tab = await mount();
    // A picture for a person reviewing the layout: `DECISIONS_SCREENSHOT_DIR=/tmp/x`.
    const shots = process.env.DECISIONS_SCREENSHOT_DIR;
    if (shots) {
      const box = await tab.locator('.br-summary-card').boundingBox();
      if (box) await tab.screenshot({ path: `${shots}/summary-decisions.png`, clip: box });
    }
    const measured = await tab.evaluate(() => {
      const card = document.querySelector('.br-summary-card')!.getBoundingClientRect();
      const body = document.querySelector<HTMLElement>('.br-summary__body')!;
      const rows = Array.from(document.querySelectorAll<HTMLElement>('.br-decisions__row'));
      return {
        card: { left: card.left, right: card.right, width: card.width },
        rows: rows.map((row) => {
          const box = row.getBoundingClientRect();
          return { height: box.height, left: box.left, right: box.right };
        }),
        bars: document.querySelectorAll('.br-decisions__bar, .br-pbar').length,
        heading: (() => {
          const line = document.querySelector('.br-decisions .br-summary__heading')!;
          const box = line.getBoundingClientRect();
          const toggle = line.querySelector('.br-summary__toggle')!.getBoundingClientRect();
          const button = line.querySelector('.br-decisions__monitors')!.getBoundingClientRect();
          return {
            height: box.height,
            toggleRight: toggle.right,
            buttonLeft: button.left,
            buttonRight: button.right,
            buttonHeight: button.height,
            buttonWidth: button.width,
          };
        })(),
        cut: rows.filter((row) => {
          const name = row.querySelector<HTMLElement>('.br-decisions__name')!;
          return name.scrollWidth > name.clientWidth;
        }).length,
        bodyScroll: { client: body.clientWidth, scroll: body.scrollWidth },
        pageScrollWidth: document.documentElement.scrollWidth,
        viewport: window.innerWidth,
      };
    });
    await tab.close();
    // Four monitors and one decide row for fifty calls: five rows for 5,000 calls. No gate
    // row (its place is its phase's chip). An errored monitor opens the section by default.
    expect(measured.rows).toHaveLength(5);
    expect(measured.card.width).toBeLessThanOrEqual(RAIL_WIDTH);
    for (const row of measured.rows) {
      // One line, the card's 28px.
      expect(row.height).toBe(28);
      // The row bleeds 8px into the 12px gutter, never past the card.
      expect(row.left).toBeGreaterThanOrEqual(measured.card.left);
      expect(row.right).toBeLessThanOrEqual(measured.card.right + 0.5);
    }
    // No probability bar anywhere in the section.
    expect(measured.bars).toBe(0);
    // The heading is one 28px line at 240px: the toggle, then the 24px Monitors button,
    // inside the card.
    expect(measured.heading.height).toBe(28);
    expect(measured.heading.buttonHeight).toBe(24);
    expect(measured.heading.buttonWidth).toBe(24);
    expect(measured.heading.toggleRight).toBeLessThanOrEqual(measured.heading.buttonLeft + 0.5);
    expect(measured.heading.buttonRight).toBeLessThanOrEqual(measured.card.right);
    // The long names were long enough to overflow, and were cut rather than widening anything.
    expect(measured.cut).toBeGreaterThan(0);
    expect(measured.bodyScroll.scroll).toBeLessThanOrEqual(measured.bodyScroll.client);
    expect(measured.pageScrollWidth).toBeLessThanOrEqual(measured.viewport);
  }, 60_000);
});
