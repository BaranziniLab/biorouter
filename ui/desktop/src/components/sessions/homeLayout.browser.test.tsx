// @vitest-environment node
/**
 * Home at real window sizes, in a real layout engine (owner, 2026-11-19).
 *
 * The owner's three observations, each a measurement jsdom cannot make:
 *  - a split pane's composer narrows a long way and stays usable, while Home's could not
 *    narrow at all (the window stopped at 1048);
 *  - so the window now narrows to `WINDOW_MIN_WIDTH` once the sidebar has folded, and Home's
 *    composer narrows with it, fluid down to that floor;
 *  - at the shortest window the usage card ran down to the composer. Now it yields first
 *    (`homeUsageFit.ts`): it shows only at its whole measure, drops its heat map where it
 *    would not leave the gap above the composer, then steps aside. The greeting never
 *    touches the composer either.
 *
 * The real `SessionInsights` (the greeting stood in, so the sentence is chosen, not drawn) and
 * the real `HomeUsageCard` are bundled with esbuild, the daemon reads replaced by fixtures;
 * `styles/main.css` is compiled by Tailwind with the classes the bundle uses and the usage and
 * composer stylesheets are appended as written. Hub's wrappers are copied here and pinned to
 * `Hub.tsx` by the first test, and the composer is ChatInput's card and footer line in the
 * shape `composerCard.browser.test.tsx` pins. The sidebar is folded at every width tested
 * (below rung 1 it is an overlay), so the window IS Home's pane.
 *
 * Set `HOME_LAYOUT_SHOTS_DIR` to also save screenshots of every case.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import postcss from 'postcss';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { fontScale } from '../../../vite-plugins/fontScale.mjs';
import {
  CHAT_COLUMN_CHROME,
  CHAT_MEASURE,
  CHAT_MIN_WIDTH,
  WINDOW_MIN_WIDTH,
} from '../Layout/yieldLadder';
import { HOME_USAGE_MIN_GAP } from './homeUsageFit';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHOTS = process.env.HOME_LAYOUT_SHOTS_DIR;

/** Hub's wrappers, exactly as `Hub.tsx` writes them (asserted below). */
const HUB = {
  root: 'biorouter-home flex h-full min-h-0 flex-col bg-background-canvas',
  content: 'biorouter-home-content min-h-0 flex-1 overflow-y-auto overflow-x-hidden',
  composer: 'biorouter-home-composer shrink-0 px-4 pb-6 sm:px-6',
  column: 'mx-auto w-full max-w-measure-chat',
};

const SHORT_GREETING = 'What question will your data answer next?';
const LONG_GREETING =
  "Which patterns will the knowledge graph unlock for tomorrow's treatments and the patients waiting on them?";

/** The daemon's reads, as fixtures: a year of activity and a model report. */
const stubs: Plugin = {
  name: 'home-stubs',
  setup(builder) {
    builder.onResolve({ filter: /utils\/homeInsightsCache$/ }, () => ({
      path: 'cache',
      namespace: 'home-stub',
    }));
    builder.onResolve({ filter: /^\.\.\/common\/Greeting$/ }, () => ({
      path: 'greeting',
      namespace: 'home-stub',
    }));
    builder.onResolve({ filter: /utils\/userAction$/ }, () => ({
      path: 'user-action',
      namespace: 'home-stub',
    }));
    builder.onResolve({ filter: /^\.\.\/\.\.\/api$/ }, () => ({
      path: 'api',
      namespace: 'home-stub',
    }));
    const modules: Record<string, string> = {
      cache: `
        export const HOME_ACTIVITY_DAYS = 371;
        export function getCachedHomeActivity() { return window.__HOME_ACTIVITY; }
        export function refreshHomeActivity() { return Promise.resolve(window.__HOME_ACTIVITY); }
      `,
      greeting: `
        import * as React from 'react';
        export function Greeting({ className }) {
          return React.createElement('h1', { className, id: 'greeting' }, window.__GREETING);
        }
      `,
      'user-action': `export async function userActionHeaders() { return {}; }`,
      api: `
        export async function getUsageReport() {
          return { data: { rows: [
            { modelId: 'gpt-5.6-sol', provider: 'openai', inputTokens: 900000, outputTokens: 100000, totalTokens: 1000000 },
          ] } };
        }
      `,
    };
    builder.onLoad({ filter: /.*/, namespace: 'home-stub' }, ({ path }) => ({
      contents: modules[path],
      loader: 'tsx',
      resolveDir: here,
    }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { SessionInsights } from './SessionsInsights';
import { composerCardClassName } from '../composer/composerCard';
import { Button } from '../ui/button';
import { ArrowUp, Folder, Mic, Plus } from '../icons/app-icons';

const HUB = ${JSON.stringify(HUB)};

/** ChatInput on Home: the one-line card, and the footer line (the folder and the model). */
function Composer({ pane = false }) {
  return (
    <div id="composer" className="relative flex w-full flex-col gap-1">
      <div id="composer-card" className={composerCardClassName({ isDraggingOver: false })}>
        <div className="br-composer-row" data-testid="chat-input-toolbar">
          <span className="br-composer-row__slot">
            <Button id="plus" type="button" size="default" shape="round" variant="ghost" aria-label="Add">
              <Plus className="size-4" />
            </Button>
          </span>
          <form className="br-composer-row__field">
            <textarea
              id="text"
              rows={1}
              aria-label="Message"
              placeholder="Ask Biorouter anything…"
              className="br-composer-input block w-full resize-none border-none bg-transparent px-0 py-1.5 text-body text-text-default placeholder:text-text-muted"
            />
          </form>
          <span className="br-composer-row__slot">
            <Button id="mic" type="button" size="sm" shape="round" variant="ghost" aria-label="Dictate">
              <Mic className="size-4" />
            </Button>
          </span>
          <span className="br-composer-row__slot">
            <Button id="send" type="button" size="default" shape="round" variant="secondary" aria-label="Send" className="br-composer-send">
              <ArrowUp className="size-4" />
            </Button>
          </span>
        </div>
      </div>
      <div id="footer" className="br-footline br-composer-footer">
        <div className="br-footline__side">
          <span id="folder" className="br-footline__item">
            <Folder className="size-3.5" aria-hidden />
            <span className="br-footline__name">home-directory-with-a-long-name</span>
          </span>
        </div>
        <div className="br-footline__side" data-side="end">
          <div className="br-footline__model">
            <button id="model" type="button" className="br-picker-chip" data-kind="model">
              <span className="br-picker-chip__name">GPT-5.6 Sol with a long model name · Deep</span>
            </button>
          </div>
          {pane ? (
            <>
              <span id="ring" className="br-footline__item">38% left</span>
              <span id="cost" className="br-footline__item">$0.42</span>
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function Home() {
  return (
    <div className={HUB.root} style={{ height: '100vh' }}>
      <div className={HUB.content}>
        <SessionInsights />
      </div>
      <div className={HUB.composer}>
        <div className={HUB.column}>
          <Composer />
        </div>
      </div>
    </div>
  );
}

/** A split pane's composer bar at the chat floor (BaseChat's 28px gutters, its pt-7 and pb-6). */
function PaneBar() {
  return (
    <div id="pane" className="bg-background-canvas" style={{ width: ${CHAT_MIN_WIDTH} }}>
      <div className="flex-shrink-0 pb-6 pt-7" style={{ paddingInline: ${CHAT_COLUMN_CHROME / 2} }}>
        <Composer pane />
      </div>
    </div>
  );
}

const root = createRoot(document.getElementById('root')!);
root.render(window.__PANE ? <PaneBar /> : <Home />);
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

/** A year of activity, with some active days so the grid is not empty. */
function year() {
  const days = [];
  for (let i = 0; i < 120; i += 3) {
    const date = new Date(Date.UTC(2026, 6, 1 + i));
    days.push({
      date: date.toISOString().slice(0, 10),
      sessions: 1 + (i % 4),
      messages: 10 + i,
      tokens: 1000 * (i + 1),
      tokensComplete: true,
      level: 1 + (i % 4),
    });
  }
  return {
    start: '2025-10-04',
    end: '2026-10-09',
    maxSessions: 4,
    maxTokens: 121000,
    tokensComplete: true,
    currentStreak: 0,
    longestStreak: 16,
    days,
  };
}

type Case = { width: number; height: number; scale?: number; greeting?: string };

describe('Home at real window sizes', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';
  let scaled = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Home layout tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [stubs],
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
    css =
      compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])) +
      readFileSync(resolve(here, '../usage/usage.css'), 'utf8') +
      readFileSync(resolve(here, '../bottom_menu/pickers.css'), 'utf8') +
      readFileSync(resolve(here, '../composer/composer.css'), 'utf8');
    scaled = (await postcss([fontScale()]).process(css, { from: undefined })).css;
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(
    c: Case,
    { mode = 'light', pane = false }: { mode?: 'light' | 'dark'; pane?: boolean } = {}
  ): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: c.width, height: c.height } });
    const scale = c.scale ?? 1;
    await tab.setContent(
      `<!doctype html><html class="${mode}" style="--app-font-scale: ${scale}"><head><style>${
        scale === 1 ? css : scaled
      }</style></head><body style="margin:0;background:var(--background-canvas)"><div id="root"></div></body></html>`
    );
    await tab.evaluate(
      ([activity, greeting, isPane]) => {
        const w = window as unknown as Record<string, unknown>;
        w.__HOME_ACTIVITY = activity;
        w.__GREETING = greeting;
        w.__PANE = isPane;
      },
      [year(), c.greeting ?? SHORT_GREETING, pane] as const
    );
    await tab.addScriptTag({ content: script });
    // `attached`: a card that has stepped aside is hidden, and is still the thing to wait for.
    await tab.waitForSelector(pane ? '#pane' : '[data-testid="home-usage-card"]', {
      state: 'attached',
    });
    // The fit answers in a layout effect and again from its observers; let them settle.
    await tab.waitForTimeout(250);
    if (SHOTS) {
      mkdirSync(SHOTS, { recursive: true });
      await tab.screenshot({
        path: `${SHOTS}/home-${pane ? 'pane-' : ''}${c.width}x${c.height}${
          scale === 1 ? '' : '-larger'
        }${c.greeting === LONG_GREETING ? '-long' : ''}-${mode}.png`,
      });
    }
    return tab;
  }

  /** Every box the claims are about, read in one pass. */
  const measure = (tab: Page) =>
    tab.evaluate(() => {
      const box = (s: string) => document.querySelector(s)!.getBoundingClientRect();
      const usage = document.querySelector<HTMLElement>('[data-testid="home-usage"]')!;
      const card = document.querySelector<HTMLElement>('[data-testid="home-usage-card"]')!;
      const grid = document.querySelector<HTMLElement>('[role="grid"]');
      const scroller = document.querySelector<HTMLElement>('.biorouter-home-content')!;
      return {
        fit: usage.dataset.fit,
        inert: usage.inert,
        column: box('.biorouter-home-column').width,
        usage: {
          top: box('[data-testid="home-usage"]').top,
          bottom: box('[data-testid="home-usage"]').bottom,
          height: box('[data-testid="home-usage"]').height,
        },
        cardVisibility: getComputedStyle(card).visibility,
        extraHeight: box('.br-usage-card-extra').height,
        gridVisibility: grid ? getComputedStyle(grid).visibility : null,
        hero: box('.biorouter-home-hero'),
        heroShown:
          getComputedStyle(document.querySelector('.biorouter-home-hero')!).display !== 'none',
        greeting: box('#greeting'),
        composer: box('#composer-card'),
        composerArea: box('.biorouter-home-composer'),
        clipped: scroller.scrollHeight - scroller.clientHeight,
      };
    });

  /** The composer's controls and footer, measured against the card they sit in. */
  const composerParts = (tab: Page) =>
    tab.evaluate(() => {
      const box = (s: string) => {
        const el = document.querySelector(s);
        return el ? el.getBoundingClientRect() : null;
      };
      const clipped = (s: string) => {
        const el = document.querySelector(s);
        return el ? el.scrollWidth > el.clientWidth + 1 : null;
      };
      return {
        card: box('#composer-card')!,
        plus: box('#plus')!,
        text: box('#text')!,
        mic: box('#mic')!,
        send: box('#send')!,
        footer: box('#footer')!,
        folder: box('#folder')!,
        model: box('#model')!,
        ring: box('#ring'),
        cost: box('#cost'),
        ringClipped: clipped('#ring'),
        costClipped: clipped('#cost'),
      };
    });

  it('copies Hub’s own wrappers', () => {
    const hub = readFileSync(resolve(here, '../Hub.tsx'), 'utf8');
    for (const className of Object.values(HUB)) expect(hub).toContain(`"${className}"`);
  });

  /**
   * The narrowest composer the app ships is a split pane's at the chat floor (rung 5): a 360px
   * pane less its 56px of chrome, a 304px card. Measured here as usable: every control on its
   * row inside the card with room to type, the footer inside it, the ring and the cost whole.
   * Home's narrowest is wider than this (`measures.test.ts`), so it is usable too.
   */
  it('measures the pane composer at the chat floor as fully usable', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ width: 800, height: 400 }, { pane: true });
    try {
      const p = await composerParts(tab);
      expect(Math.round(p.card.width)).toBe(CHAT_MIN_WIDTH - CHAT_COLUMN_CHROME);
      for (const control of [p.plus, p.text, p.mic, p.send]) {
        expect(control.left).toBeGreaterThanOrEqual(p.card.left);
        expect(control.right).toBeLessThanOrEqual(p.card.right);
      }
      expect(p.plus.right).toBeLessThanOrEqual(p.text.left);
      expect(p.text.right).toBeLessThanOrEqual(p.mic.left);
      expect(p.mic.right).toBeLessThanOrEqual(p.send.left);
      // Room for a few words: the field keeps more than half the card.
      expect(p.text.width).toBeGreaterThan(p.card.width / 2);
      for (const item of [p.folder, p.model, p.ring!, p.cost!]) {
        expect(item.left).toBeGreaterThanOrEqual(p.footer.left - 0.5);
        expect(item.right).toBeLessThanOrEqual(p.footer.right + 0.5);
        expect(item.width).toBeGreaterThan(0);
      }
      expect(p.ringClipped).toBe(false);
      expect(p.costClipped).toBe(false);
    } finally {
      await tab.close();
    }
  });

  /**
   * A grid of windows, the sidebar folded: the composer is fluid down to the floor and
   * centred, the card shows only at its whole measure, and whatever shows above the
   * composer leaves it the gap.
   */
  const GRID: Case[] = [];
  for (const width of [WINDOW_MIN_WIDTH, 760, 807, 808, 900, 1048])
    for (const height of [600, 700, 900]) GRID.push({ width, height });

  it.for(GRID)('lays Home out at $width x $height', async ({ width, height }, ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ width, height });
    try {
      const m = await measure(tab);
      // Fluid down to the floor: the composer is the window less Home's gutters, up to the measure.
      expect(m.composer.width).toBeCloseTo(Math.min(CHAT_MEASURE, width - 2 * 24), 0);
      expect(m.composer.left).toBeCloseTo((width - m.composer.width) / 2, 0);
      expect(m.column).toBeCloseTo(m.composer.width, 0);

      if (width < CHAT_MEASURE + 2 * 24) {
        expect(m.fit, 'narrower than the measure').toBe('hidden');
      } else {
        expect(m.fit).not.toBe('hidden');
      }

      // The heading and the card never touch the composer.
      expect(m.composer.top - m.hero.bottom).toBeGreaterThanOrEqual(HOME_USAGE_MIN_GAP - 0.5);
      if (m.fit !== 'hidden') {
        expect(m.composer.top - m.usage.bottom).toBeGreaterThanOrEqual(HOME_USAGE_MIN_GAP - 0.5);
      }
      // And nothing above the composer is cut off by its edge.
      expect(m.clipped).toBeLessThanOrEqual(0);
    } finally {
      await tab.close();
    }
  });

  it('keeps the whole card at the shortest window with the standard text size', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ width: 1048, height: 600 });
    try {
      const m = await measure(tab);
      expect(m.fit).toBe('full');
      expect(m.gridVisibility).toBe('visible');
      expect(m.composer.top - m.usage.bottom).toBeGreaterThanOrEqual(HOME_USAGE_MIN_GAP);
    } finally {
      await tab.close();
    }
  });

  /**
   * The owner's case: the shortest window, the Larger text size and a greeting on two lines.
   * The whole card would run down to the composer, so the heat map folds away and the tiles
   * stay, with the gap above the composer.
   */
  it('drops the heat map first at the shortest window with Larger text', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(
        { width: 1048, height: 600, scale: 1.15, greeting: LONG_GREETING },
        { mode }
      );
      try {
        const m = await measure(tab);
        expect(m.greeting.height, 'the greeting is on two lines').toBeGreaterThan(40);
        expect(m.fit).toBe('compact');
        expect(m.extraHeight).toBe(0);
        expect(m.gridVisibility).toBe('hidden');
        expect(m.cardVisibility).toBe('visible');
        expect(m.composer.top - m.usage.bottom).toBeGreaterThanOrEqual(HOME_USAGE_MIN_GAP);
        expect(m.clipped).toBeLessThanOrEqual(0);
      } finally {
        await tab.close();
      }
    }
  });

  /**
   * Shorter still (a `biorouter serve` browser has no window floor): the tiles would not leave
   * the gap either, so the card steps aside, out of sight, out of reach, and out of the flow,
   * and the greeting stays clear of the composer. (Shorter again, under a 340px room, the
   * greeting itself folds away as it always has, and the tiles have room once more.)
   */
  it('steps the card aside where even its tiles would touch the composer', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ width: 1048, height: 480, scale: 1.15, greeting: LONG_GREETING });
    try {
      const m = await measure(tab);
      expect(m.fit).toBe('hidden');
      expect(m.usage.height).toBe(0);
      expect(m.cardVisibility).toBe('hidden');
      expect(m.inert).toBe(true);
      expect(m.composer.top - m.hero.bottom).toBeGreaterThanOrEqual(HOME_USAGE_MIN_GAP);
    } finally {
      await tab.close();
    }
  });

  it('brings the card back whole when the window grows again', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ width: WINDOW_MIN_WIDTH, height: 600 });
    try {
      expect((await measure(tab)).fit).toBe('hidden');
      await tab.setViewportSize({ width: 1048, height: 900 });
      await tab.waitForTimeout(250);
      const m = await measure(tab);
      expect(m.fit).toBe('full');
      expect(m.inert).toBe(false);
      expect(m.gridVisibility).toBe('visible');
    } finally {
      await tab.close();
    }
  });
});
