// @vitest-environment node
/**
 * Settings > App > Usage when a person comes back to the App tab, measured in a real browser
 * (vision V4-R7-3, principle 9: a surface mounted by a tab switch appears at rest).
 *
 * Settings mounts only the tab on screen, so every return to App mounts `UsageSection` afresh and
 * the section used to start from nothing: a 712x40 pulsing skeleton for the 90 to 190 ms the
 * daemon took to answer, then the row. It sat below the fold on most displays and showed on a
 * taller one. The section now starts from the last month-to-date read of this process
 * (`usageSnapshot.ts`) and replaces it in place with the daemon's answer.
 *
 * jsdom cannot see a frame that was painted, only the DOM after React settled, so this takes the
 * two views the Model row's test takes (`modelsGeometry.browser.test.tsx`, V4-R7-1): `commits`
 * samples the DOM at the end of every task that changed it, so no state React committed is
 * skipped, and `frames` samples it in every animation frame, the state about to be painted.
 *
 * The real `UsageSection` is bundled with esbuild (the generated API client and the user-action
 * header stubbed), unmounted and mounted again in Chromium the way a tab switch does it, and
 * styled by the authored `@ws WS-PRIMITIVES` blocks of `styles/main.css`. Tailwind is not run, so
 * only the utilities the measured boxes depend on are written out below.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../../styles/main.css'), 'utf8');

const TOKEN_NAMES = [
  '--control-md',
  '--control-sm',
  '--radius-full',
  '--radius-element',
  '--radius-inner',
  '--dur-fast-min',
  '--dur-fast',
  '--dur-fast-max',
  '--ease-out',
  '--ease-spring',
  '--row-height',
  '--radius-md',
  '--font-mono',
];

/** The first declaration of each token in main.css, so a token change reaches this test. */
function readTokens(): string {
  const lines = TOKEN_NAMES.map((name) => {
    const match = MAIN_CSS.match(new RegExp(`\\n\\s*${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
    if (!match) throw new Error(`main.css does not declare ${name}`);
    return `  ${name}: ${match[1].trim()};`;
  });
  return `:root {\n${lines.join('\n')}\n}`;
}

/** Every `@ws WS-PRIMITIVES` block of main.css, in file order. */
function primitiveBlocks(): string {
  const out: string[] = [];
  const re =
    /\/\* @ws WS-PRIMITIVES begin: ([^*]+?) \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: \1 \*\//g;
  for (const match of MAIN_CSS.matchAll(re)) out.push(match[2]);
  if (out.length === 0) throw new Error('main.css has no @ws WS-PRIMITIVES block');
  return out.join('\n');
}

/** Tailwind's preflight, the roles and the few utilities the measured boxes use. */
const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
button { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
h2, h3, p { margin: 0; font-size: inherit; font-weight: inherit; }
svg { display: block; }
button svg { width: 16px; height: 16px; flex: none; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
.text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
.text-secondary { font-size: 13px; line-height: 18px; }
.text-supporting, .text-caps { font-size: 12px; line-height: 16px; }
.tabular-nums { font-variant-numeric: tabular-nums; }
.flex { display: flex; } .inline-flex { display: inline-flex; } .items-center { align-items: center; }
.justify-center { justify-content: center; } .min-w-0 { min-width: 0; } .shrink-0 { flex-shrink: 0; }
.gap-1\\.5 { gap: 6px; } .gap-2 { gap: 8px; } .whitespace-nowrap { white-space: nowrap; }
.px-3 { padding-inline: 12px; } .mb-2 { margin-bottom: 8px; }
.rounded-element { border-radius: 8px; }
.h-control-sm { height: var(--control-sm); }
.h-10 { height: 40px; } .w-full { width: 100%; }
.animate-pulse { animation: pulse 2s cubic-bezier(0.4, 0, 0.6, 1) infinite; }
@keyframes pulse { 50% { opacity: 0.5; } }
.bg-background-muted { background: var(--background-muted); }
.biorouter-settings-list { display: flex; flex-direction: column; }
.biorouter-settings-section-header { margin-bottom: 8px; }
.biorouter-settings-section { padding-block: 14px; }
:root { --control-track-off: #ebebe8; --background-accent: #b85a32; --border-control: #86847f;
  --background-default: #fff; --text-on-accent: #fff; --text-muted: #666; --text-default: #111;
  --text-subtle: #777; --text-danger: #b00020; --background-muted: #f2f2f0;
  --border-focus: #555; --background-focus: #ddd; --control-sm: 28px;
  --shadow-raised: 0 1px 2px rgb(0 0 0 / 0.1); --inset-hairline: inset 0 0 0 1px #ddd;
  --overlay-hover: rgb(0 0 0 / 0.05); --border-subtle: #ccc; }
`;

/** The turns the daemon reports, so one answer can be told from another. */
const FIRST_TURNS = 3;
const LATER_TURNS = 12;

const STUBS: Array<{ filter: RegExp; contents: string }> = [
  {
    filter: /\/api$/,
    contents: `
      // The daemon's three month-to-date reads. A page sets __daemonDelayMs to make them slow, so
      // anything on screen before they answer can only have come from what the section held.
      const totals = (turns) => ({
        inputTokens: 1000, outputTokens: 200, totalTokens: 1200,
        cacheReadTokens: 0, cacheCreationTokens: 0, turns, cost: 0.5,
        hasUnpriced: false, costExcludesCache: false,
      });
      const late = async (value) => {
        await new Promise((done) => setTimeout(done, window.__daemonDelayMs || 0));
        return { data: value };
      };
      export const getUsageSummary = () => {
        const turns = window.__turns;
        return late({
          month: '2026-10', monthToDate: totals(turns), allTime: totals(turns),
          monthlyTokenLimit: null, monthlyDollarLimit: null, tokenPercent: null, dollarPercent: null,
        });
      };
      export const getUsageReport = () => late({ rows: [] });
    `,
  },
  { filter: /\/userAction$/, contents: `export const userActionHeaders = async () => ({});` },
];

const stubPlugin: Plugin = {
  name: 'stubs',
  setup(b) {
    for (const { filter, contents } of STUBS) {
      b.onResolve({ filter }, (args) =>
        // Only the specifiers the components under test import; the stub files do not exist.
        args.importer.includes('/settings/')
          ? { path: args.path, namespace: 'stub', pluginData: contents }
          : undefined
      );
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      loader: 'js',
    }));
    // The stylesheets are added to the page by hand.
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import UsageSection from './UsageSection';

// A tab switch unmounts the whole App tab and mounts it again, so each visit is a new root.
let root = null;
window.__mount = () => {
  root = createRoot(document.getElementById('root'));
  root.render(
    <div style={{ width: 712, padding: 16 }}>
      <UsageSection />
    </div>
  );
};
window.__unmount = () => {
  root.unmount();
  root = null;
};
`;

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

type Sample = {
  section: boolean;
  at: number;
  panel: boolean;
  text: string | null;
  skeleton: boolean;
  pulse: boolean;
  status: boolean;
  sectionTop: number | null;
  sectionHeight: number | null;
  rowTop: number | null;
  rowHeight: number | null;
};
type Probe = { commits: Sample[]; frames: Sample[] };

/** Every sample from the first one that has the row. */
const fromRow = (samples: Sample[]) => samples.slice(samples.findIndex((sample) => sample.panel));

/**
 * Installed once, before the first visit mounts. `window.__resetProbe()` starts a fresh record
 * for the next visit; the sampling itself never stops, so no commit or frame goes unseen.
 */
const PROBE = `
  window.__probe = { commits: [], frames: [] };
  window.__resetProbe = () => { window.__probe = { commits: [], frames: [] }; };
  const sample = () => {
    const section = document.querySelector('#settings-usage');
    const panel = document.querySelector('[data-testid="usage-panel"]');
    const row = panel && panel.querySelector('.br-setting-row');
    const s = section && section.getBoundingClientRect();
    const r = row && row.getBoundingClientRect();
    return {
      at: performance.now(),
      section: Boolean(section),
      panel: Boolean(panel),
      text: panel ? panel.textContent : null,
      skeleton: Boolean(document.querySelector('[data-testid="usage-loading"]')),
      pulse: Boolean(document.querySelector('.animate-pulse')),
      status: Boolean(document.querySelector('[role="status"]')),
      sectionTop: s ? s.top : null,
      sectionHeight: s ? s.height : null,
      rowTop: r ? r.top : null,
      rowHeight: r ? r.height : null,
    };
  };
  new MutationObserver(() => window.__probe.commits.push(sample())).observe(document.body, {
    subtree: true, childList: true, characterData: true, attributes: true,
  });
  const tick = () => {
    window.__probe.frames.push(sample());
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
`;

const EPSILON = 0.5;

describe('Settings > App > Usage on a return to the tab (V4-R7-3)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the return visit.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [stubPlugin],
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css = PREFLIGHT + readTokens() + primitiveBlocks();
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function newPage(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 1000 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: PROBE });
    await tab.addScriptTag({ content: script });
    return tab;
  }

  /** Make the daemon answer in `delayMs` with `turns` turns, from the next request on. */
  const daemon = (tab: Page, delayMs: number, turns: number) =>
    tab.evaluate(
      ([delay, count]) => {
        const w = window as unknown as Record<string, unknown>;
        w.__daemonDelayMs = delay;
        w.__turns = count;
      },
      [delayMs, turns]
    );

  /** The record of the visit so far. */
  const probeOf = (tab: Page) =>
    tab.evaluate(() => (window as unknown as { __probe: Probe }).__probe);

  /** Two animation frames: whatever React committed has been painted. */
  const painted = (tab: Page) =>
    tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );

  const mount = (tab: Page) =>
    tab.evaluate(() => (window as unknown as { __mount: () => void }).__mount());
  const leave = (tab: Page) =>
    tab.evaluate(() => (window as unknown as { __unmount: () => void }).__unmount());
  const startRecord = (tab: Page) =>
    tab.evaluate(() => (window as unknown as { __resetProbe: () => void }).__resetProbe());

  it('starts the very first read as a skeleton the height of the row, so the one swap moves nothing', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await newPage();
    try {
      await daemon(tab, 150, FIRST_TURNS);
      await startRecord(tab);
      await mount(tab);
      await tab.waitForSelector('[data-testid="usage-panel"]');
      await painted(tab);
      const probe = await probeOf(tab);

      // The instrument can see a skeleton: it is there in the first commit that has the section.
      const firstCommit = probe.commits.find((sample) => sample.section)!;
      expect(firstCommit.skeleton, 'the first read starts as the skeleton').toBe(true);
      expect(firstCommit.pulse).toBe(true);
      expect(firstCommit.panel).toBe(false);

      const skeleton = probe.frames.find((sample) => sample.skeleton)!;
      const row = probe.frames.find((sample) => sample.panel)!;
      expect(skeleton, 'a painted skeleton frame').toBeTruthy();
      expect(Math.abs(skeleton.sectionHeight! - row.sectionHeight!)).toBeLessThanOrEqual(EPSILON);
      expect(Math.abs(skeleton.sectionTop! - row.sectionTop!)).toBeLessThanOrEqual(EPSILON);
    } finally {
      await tab.close();
    }
  });

  it('draws the row in the first commit and every frame of a return, then swaps the answer in place', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await newPage();
    try {
      // First visit of the process, then leave: Settings goes to another tab.
      await daemon(tab, 100, FIRST_TURNS);
      await mount(tab);
      await tab.waitForSelector('[data-testid="usage-panel"]');
      await leave(tab);
      await painted(tab);

      // Come back. The daemon is slow (600 ms) and has a different answer, so everything painted
      // before it lands is the section's own, and the swap when it does is visible.
      await daemon(tab, 600, LATER_TURNS);
      await startRecord(tab);
      await mount(tab);
      await tab.waitForFunction(
        (turns) => {
          const probe = (window as unknown as { __probe: Probe }).__probe;
          const last = probe.frames[probe.frames.length - 1];
          return Boolean(last?.text?.includes(`${turns} turns`));
        },
        LATER_TURNS,
        { timeout: 10_000 }
      );
      // Long enough for the row to settle after the answer, so a late status line would show.
      await new Promise((done) => setTimeout(done, 300));
      await painted(tab);
      const probe = await probeOf(tab);

      const commits = probe.commits.filter((sample) => sample.section);
      const frames = fromRow(probe.frames);
      expect(commits.length).toBeGreaterThan(0);
      expect(frames.length).toBeGreaterThan(20);

      // Not one commit and not one frame with the section is a skeleton, a pulse or a status line.
      for (const sample of [...commits, ...frames]) {
        expect(sample.panel, 'the row is drawn whenever the section is').toBe(true);
        expect(sample.skeleton, 'a skeleton').toBe(false);
        expect(sample.pulse, 'a pulsing box').toBe(false);
        expect(sample.status, 'a "Refreshing usage" line').toBe(false);
      }

      // The first words are the seed's; the daemon's replace them about 600 ms later, in place.
      expect(commits[0].text).toContain(`${FIRST_TURNS} turns`);
      expect(frames[0].text).toContain(`${FIRST_TURNS} turns`);
      const landed = frames.find((sample) => sample.text?.includes(`${LATER_TURNS} turns`))!;
      expect(landed.at - frames[0].at).toBeGreaterThan(450);
      for (const sample of frames) {
        expect(
          sample.text?.includes(`${FIRST_TURNS} turns`) ||
            sample.text?.includes(`${LATER_TURNS} turns`)
        ).toBe(true);
      }

      // And nothing moves: the section and its row are in the same place in every frame.
      const at = frames[0];
      for (const sample of frames) {
        expect(Math.abs(sample.sectionTop! - at.sectionTop!)).toBeLessThanOrEqual(EPSILON);
        expect(Math.abs(sample.sectionHeight! - at.sectionHeight!)).toBeLessThanOrEqual(EPSILON);
        expect(Math.abs(sample.rowTop! - at.rowTop!)).toBeLessThanOrEqual(EPSILON);
        expect(Math.abs(sample.rowHeight! - at.rowHeight!)).toBeLessThanOrEqual(EPSILON);
      }
    } finally {
      await tab.close();
    }
  });
});
