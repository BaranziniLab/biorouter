// @vitest-environment node
/**
 * The footer cards under a real pointer and a real layout, in Chromium, for ALL THREE items on the
 * line (the working folder, the context ring and the cost):
 *   - each card opens on a hover after the delay, and a pointer that only passes over its trigger
 *     opens nothing;
 *   - each hangs ABOVE THE WHOLE COMPOSER card, never over the draft, `+` or Tools, nor over the
 *     trigger it came from, lined up with the folder chip's left edge or with a readout's right
 *     edge, and below the window-drag band even in a short window; everything it says, a long path
 *     included, is drawn inside it;
 *   - the pointer can travel from any trigger up across the composer into its card, and from the
 *     card back down to its trigger, without losing it, at a hand's pace; a pointer that stops on
 *     the composer or heads elsewhere closes it; a click into the draft right after a hover places
 *     the caret rather than pinning the card;
 *   - the three cards wear ONE surface: the same width, padding, radius, ground, hairline and
 *     shadow, and the same label, value and detail type; and a card holding keyboard focus shows
 *     it, each the same;
 *   - HOVER PEEKS, CLICK PINS on the three triggers alike: at rest, with the pointer on it, while
 *     a hover only previews its card (the rest look), and KEPT by a click (the strong fill, held
 *     with the pointer off it and back on it), which also draws an edge in forced colours; a kept
 *     ring keeps the circle of its track;
 *   - only one card is open at a time, and a hover opens nothing over a kept one.
 * The same rules' behaviour in jsdom, item by item, is `footerCards.contract.test.tsx`.
 *
 * jsdom cannot say any of this: it has no pointer, dispatches no enter or leave as a pointer
 * crosses a gap, and places nothing, so Radix's positioning and the authored geometry in
 * `pickers.css` never meet. So the REAL `ComposerFooter` (with the real `DirSwitcher`,
 * `ContextWindowIndicator`, `CostTracker`, `useFooterCard`, `FooterCard` and `Popover`) is bundled
 * with esbuild and rendered in Chromium under a composer stand-in, wired as `ChatInput` wires it.
 * Only the daemon's API, the config store, the model context, the pricing fetch, the toasts and the
 * proof-of-user headers are stand-ins.
 *
 * ⚠ Tailwind does not run here, so a utility's paint is absent. The surface's two utilities that
 * the comparison below reads (`rounded-container`, `bg-background-default`, from `PopoverContent`)
 * are mapped to the tokens Tailwind maps them to, and the hairline and shadow are main.css's own
 * `.biorouter-popover-surface` rule. Everything else is authored CSS (`.br-footer-card*`).
 */
import { readFileSync } from 'node:fs';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { FOOTER_CARD_CLOSE_GRACE_MS, FOOTER_CARD_OPEN_DELAY_MS } from './useFooterCard';

const here = dirname(fileURLToPath(import.meta.url));
const SRC = resolve(here, '../..');
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const PICKERS_CSS = readFileSync(resolve(here, 'pickers.css'), 'utf8');

const TOKEN_NAMES = [
  // The kept trigger's fill, and the neutrals the context ring's track is measured in.
  '--background-fill-strong',
  '--color-neutral-200',
  '--color-neutral-300',
  '--border-strong',
  '--control-compact',
  '--control-sm',
  '--control-md',
  '--radius-element',
  '--radius-container',
  '--icon-chip',
  '--text-supporting',
  '--text-supporting--line-height',
  '--text-secondary',
  '--text-secondary--line-height',
  '--text-label',
  '--text-label--line-height',
  '--dur-fast-min',
  '--dur-med-min',
  '--ease-out',
  '--shadow-popover',
];

/** A token's first declaration in main.css (its light, base value), as written. */
function declaration(name: string): string {
  const match = MAIN_CSS.match(new RegExp(`\\n\\s*${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
  if (!match) throw new Error(`main.css does not declare ${name}`);
  return match[1].trim();
}

/** The first declaration of each token in main.css, so a token change reaches this test. */
function readTokens(): string {
  const lines = TOKEN_NAMES.map((name) => `  ${name}: ${declaration(name)};`);
  return `:root {\n${lines.join('\n')}\n}`;
}

/**
 * The context ring's track is measured against the real neutrals, so on the footer line (and
 * only there: the cards' hairline keeps the stand-in below) the subtle border is main.css's own.
 * With the stand-in, a far darker grey, the track would read on the kept fill whatever
 * pickers.css said, and the measurement could not fail.
 */
const REAL_LINE_NEUTRALS = `.br-footline { --border-subtle: ${declaration('--border-subtle')}; }`;

/** One of main.css's own rules, verbatim (its first, light, declaration). */
function readRule(selector: string): string {
  const match = MAIN_CSS.match(new RegExp(`\\n${selector.replace(/[.-]/g, '\\$&')} \\{[^}]*\\}`));
  if (!match) throw new Error(`main.css does not declare ${selector}`);
  return match[0];
}

/** Tailwind's preflight, the parts the layout depends on, and the two surface utilities. */
const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
button, input { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
p, ul { margin: 0; }
svg { display: block; width: 14px; height: 14px; }
.block { display: block; }
.font-mono { font-family: monospace; }
:root { --text-muted: rgb(102, 102, 102); --text-default: rgb(17, 17, 17);
  --overlay-hover: rgba(0, 0, 0, 0.05); --background-default: rgb(255, 255, 255);
  --background-focus: #ddd; --border-focus: rgb(85, 85, 85); --border-subtle: rgb(204, 204, 204);
  --text-danger: #a00; }
.rounded-container { border-radius: var(--radius-container); }
.bg-background-default { background-color: var(--background-default); }
/* The xs button's height (Button's \`xs\` size), which the card's row action centres on. */
.h-control-compact { height: var(--control-compact); }
/* Every Button's type role, 14/20/500, which a card's button must not keep. */
.text-label { font-size: var(--text-label); line-height: var(--text-label--line-height); font-weight: 500; }
`;

/**
 * A path the card can only fit by breaking inside a word: its last segment (a content hash, as a
 * cache names its entries) has no break opportunity in it and is far wider than the card. A path
 * of short segments wraps at its separators (`.br-footer-card__path`), which the folder tests
 * below measure.
 */
const LONG_PATH = `/Volumes/shared/cache/${'7f3a9c2e'.repeat(12)}`;

const GRANTS = [
  { id: 'grant-1', folder: '/Users/someone/data/results', access: 'read-only', descendants: true },
  { id: 'grant-2', folder: LONG_PATH, access: 'read/write', descendants: false },
];

/**
 * Everything the line needs that is not UI, keyed by the module's path under `src/`, so a stub
 * replaces it whichever file imports it and however that file spells the path. A page may list
 * other grants by setting `window.__FOLDER_GRANTS__` before the card opens (`load`'s `setup`).
 */
const STUBS: Record<string, string> = {
  api: `
    export const listFolderAccess = async () => ({
      data: window.__FOLDER_GRANTS__ ?? ${JSON.stringify(GRANTS)},
    });
    export const revokeFolderAccess = async () => ({ data: { revoked: true } });
    export const updateWorkingDir = async () => ({ data: {} });
    export const getSessionUsage = async () => ({ data: undefined });
    export const getPricing = async () => ({ data: undefined });
  `,
  toasts: 'export const toastError = () => {};',
  'utils/userAction': 'export const userActionHeaders = async () => ({});',
  'utils/pricing': 'export const fetchModelPricing = async () => null;',
  // No threshold saved: the marker sits at the default 80%.
  'components/ConfigContext': `export const useConfig = () => ({
    read: async () => null,
    upsert: async () => undefined,
  });`,
  'components/ModelAndProviderContext': `export const useModelAndProvider = () => ({
    currentModel: 'gpt-5.6-sol',
    currentProvider: 'versa_azure',
  });`,
};

const stubDaemon: Plugin = {
  name: 'stub-daemon',
  setup(builder) {
    builder.onResolve({ filter: /^\.\.?\// }, (args) => {
      const key = relative(SRC, resolve(args.resolveDir, args.path)).split('\\').join('/');
      return key in STUBS ? { path: key, namespace: 'daemon-stub' } : undefined;
    });
    builder.onLoad({ filter: /.*/, namespace: 'daemon-stub' }, (args) => ({
      contents: STUBS[args.path],
      loader: 'ts',
      resolveDir: here,
    }));
  },
};

/**
 * A chat's composer near the window's bottom edge: the composer card holding a draft, and under
 * it the real footer line, as `ChatInput` lays them out (a column with a 4px gap) and wires them
 * (the composer card handed over as `composerRef`). A chat with messages, a known folder (a page
 * may name another by setting `window.__WORKING_DIR__`, `load`'s `setup`), a loaded window with
 * tokens and a cost above $0, so all three items are on the line.
 */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ComposerFooter } from './ComposerFooter';

function App() {
  const composer = React.useRef(null);
  return (
    <div
      style={{
        position: 'fixed', left: 40, right: 40, bottom: 24,
        display: 'flex', flexDirection: 'column', gap: 4,
      }}
    >
      <div
        ref={composer}
        data-testid="composer"
        style={{ height: 110, border: '1px solid #ccc', padding: '10px 12px 10px 16px' }}
      >
        <textarea data-testid="draft" style={{ display: 'block', width: '100%', height: 60 }} />
      </div>
      <ComposerFooter
        composerRef={composer}
        sessionId="chat-1"
        workingDir={
          (window as unknown as { __WORKING_DIR__?: string }).__WORKING_DIR__ ??
          '/Users/someone/Projects/kdps'
        }
        workingDirLocked
        onWorkingDirChange={() => {}}
        totalTokens={92000}
        tokenLimit={128000}
        isTokenLimitLoaded
        onCompact={() => {}}
        modelCostRows={[
          {
            provider: 'versa_azure', model: 'gpt-5.6-sol', inputTokens: 20642, outputTokens: 198,
            cacheReadTokens: 0, cacheCreationTokens: 0, totalTokens: 20840, turns: 3,
            totalCost: 0.11, costIsPartial: false,
          },
        ]}
      />
      {/* What follows the line in the tab order, so a Tab that leaves a card has somewhere to land. */}
      <button type="button" data-testid="after-line" style={{ position: 'fixed', top: 8, left: 8 }}>
        After the line
      </button>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
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

type Box = { top: number; bottom: number; left: number; right: number; width: number };

interface Item {
  name: 'folder' | 'context' | 'cost';
  trigger: string;
  card: string;
  /** Which of the trigger's edges the card lines up with. */
  align: 'start' | 'end';
  /**
   * What a Tab from the card's own surface reaches: its first control, or, for a card that holds
   * none, whatever follows the trigger (the card closes and the Tab carries on from there).
   */
  afterTab: { role: string; name: string; closes: boolean };
  /** What a press inside the card can land on (selectors inside it): its text, and the gauge's. */
  presses: string[];
}

const ITEMS: Item[] = [
  {
    name: 'folder',
    trigger: '[data-testid="dir-switcher-locked"]',
    card: '[data-testid="folder-access-card"]',
    align: 'start',
    afterTab: { role: 'button', name: `Revoke access to ${GRANTS[0].folder}`, closes: false },
    presses: ['.br-footer-card__label', 'li .br-footer-card__value'],
  },
  {
    name: 'context',
    trigger: '[data-testid="context-window-indicator"]',
    card: '[data-testid="context-card"]',
    align: 'end',
    afterTab: { role: 'slider', name: 'Auto-compact threshold', closes: false },
    presses: ['.br-footer-card__label', '.br-context-gauge__bar', '.br-context-gauge__marker'],
  },
  {
    name: 'cost',
    trigger: '[data-testid="chat-cost"]',
    card: '[data-testid="cost-card"]',
    align: 'end',
    afterTab: { role: 'button', name: 'After the line', closes: true },
    presses: ['.br-footer-card__label', '.br-footer-card__value'],
  },
];

const [FOLDER, CONTEXT, COST] = ITEMS;
const COMPOSER = '[data-testid="composer"]';
const DRAFT = '[data-testid="draft"]';

describe('the footer cards under a real pointer', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the card tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [stubDaemon],
      loader: { '.css': 'empty' },
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css =
      PREFLIGHT +
      readTokens() +
      readRule('.biorouter-popover-surface') +
      readRule('.br-progress') +
      REAL_LINE_NEUTRALS +
      PICKERS_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** `setup` is a script the page runs before the line mounts (a config, the grants listed). */
  async function load(viewport = { width: 900, height: 720 }, setup?: string): Promise<Page> {
    const tab = await browser!.newPage({ viewport });
    // The open keyframe moves the surface; measure the layout, not the motion.
    await tab.emulateMedia({ reducedMotion: 'reduce' });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    // A page set from a string has an opaque origin, where reading `localStorage` throws, and the
    // cost reads its `show_pricing` setting there: give the page a store of its own first.
    await tab.evaluate(() => {
      const store = new Map<string, string>();
      Object.defineProperty(window, 'localStorage', {
        configurable: true,
        value: {
          getItem: (key: string) => store.get(key) ?? null,
          setItem: (key: string, value: string) => store.set(key, String(value)),
          removeItem: (key: string) => store.delete(key),
          clear: () => store.clear(),
          key: (index: number) => [...store.keys()][index] ?? null,
          get length() {
            return store.size;
          },
        },
      });
    });
    if (setup) await tab.addScriptTag({ content: setup });
    await tab.addScriptTag({ content: script });
    for (const item of ITEMS) await tab.waitForSelector(item.trigger);
    return tab;
  }

  const box = (tab: Page, selector: string) =>
    tab.evaluate((sel) => {
      const r = document.querySelector(sel)!.getBoundingClientRect();
      return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, width: r.width };
    }, selector) as Promise<Box>;

  const cardState = (tab: Page, item: Item) =>
    tab.evaluate(
      (sel) => document.querySelector(sel)?.getAttribute('data-state') ?? 'absent',
      item.card
    );

  const centre = (b: Box) => ({ x: (b.left + b.right) / 2, y: (b.top + b.bottom) / 2 });

  /** Move the pointer like a hand: `steps` moves, `pause` ms apart. */
  async function glide(
    tab: Page,
    from: { x: number; y: number },
    to: { x: number; y: number },
    steps: number,
    pause: number
  ) {
    for (let i = 1; i <= steps; i++) {
      await tab.mouse.move(
        from.x + ((to.x - from.x) * i) / steps,
        from.y + ((to.y - from.y) * i) / steps
      );
      await tab.waitForTimeout(pause);
    }
  }

  /** The card is open and has what it reads: the folder's grants, once they are listed. */
  async function waitOpen(tab: Page, item: Item) {
    await tab.waitForSelector(`${item.card}[data-state="open"]`, { timeout: 2000 });
    if (item === FOLDER) await tab.waitForSelector(`${item.card} li`);
  }

  async function hoverOpen(tab: Page, item: Item) {
    const trigger = await box(tab, item.trigger);
    const onTrigger = centre(trigger);
    await tab.mouse.move(onTrigger.x, onTrigger.y);
    await waitOpen(tab, item);
    return {
      trigger,
      onTrigger,
      card: await box(tab, item.card),
      composer: await box(tab, COMPOSER),
    };
  }

  /**
   * Hover the card open, move the pointer up across the composer into it at a hand's pace, and
   * rest there past the grace: where a person reading the card has the pointer.
   */
  async function restInCard(tab: Page, item: Item) {
    const opened = await hoverOpen(tab, item);
    const { card } = opened;
    const inCard = {
      x: item.align === 'start' ? card.left + 40 : card.right - 40,
      y: card.bottom - 20,
    };
    await glide(tab, opened.onTrigger, inCard, 10, 40);
    await tab.waitForTimeout(FOOTER_CARD_CLOSE_GRACE_MS * 3);
    expect(await cardState(tab, item), 'the pointer rests in the card').toBe('open');
    return { ...opened, inCard };
  }

  const waitClosed = (tab: Page, item: Item) =>
    tab.waitForFunction(
      (sel) => document.querySelector(sel)?.getAttribute('data-state') !== 'open',
      item.card,
      { timeout: 2000 }
    );

  const activeTestId = (tab: Page) =>
    tab.evaluate(() =>
      document.activeElement?.closest('[data-testid]')?.getAttribute('data-testid')
    );

  for (const item of ITEMS) {
    describe(`the ${item.name} card`, () => {
      it('opens on hover and hangs above the whole composer, covering neither the draft nor its trigger', async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load();
        try {
          const trigger = await box(tab, item.trigger);
          const onTrigger = centre(trigger);
          await tab.mouse.move(onTrigger.x, onTrigger.y);

          // Not before the delay...
          await tab.waitForTimeout(FOOTER_CARD_OPEN_DELAY_MS / 2);
          expect(await cardState(tab, item)).toBe('absent');
          // ...and open after it.
          await waitOpen(tab, item);

          // Above the composer card, never over it, and on the trigger's own edge.
          const card = await box(tab, item.card);
          const composer = await box(tab, COMPOSER);
          expect(card.bottom, 'the card ends above the composer').toBeLessThanOrEqual(
            composer.top + 0.5
          );
          expect(composer.top - card.bottom, 'a small gap, not a jump').toBeLessThanOrEqual(12);
          if (item.align === 'start') {
            expect(Math.abs(card.left - trigger.left), 'on the trigger’s left').toBeLessThanOrEqual(
              1
            );
          } else {
            expect(
              Math.abs(card.right - trigger.right),
              'on the trigger’s right'
            ).toBeLessThanOrEqual(1);
          }
          expect(card.width, 'the authored width').toBeCloseTo(320, 0);
          expect(card.top, 'clear of the window-drag band').toBeGreaterThanOrEqual(52);
          // What is under the trigger and under the draft is still the trigger and the draft.
          const under = (point: { x: number; y: number }) =>
            tab.evaluate(
              ({ x, y }) =>
                document
                  .elementFromPoint(x, y)
                  ?.closest('[data-testid]')
                  ?.getAttribute('data-testid'),
              point
            );
          expect(await under(onTrigger)).toBe(item.trigger.match(/"(.+)"/)![1]);
          const draft = await box(tab, DRAFT);
          expect(await under({ x: draft.left + 20, y: draft.top + 10 })).toBe('draft');
          expect(await under({ x: draft.right - 20, y: draft.top + 10 })).toBe('draft');
          expect(await under(centre(draft))).toBe('draft');
          // What a person sees stays inside the card: every line of its text, measured where the
          // text itself is drawn (a Range over each text node), against the card's inner box. An
          // element's own box cannot say this, since a line that does not wrap runs out of the box
          // that holds it while the box keeps the card's width. Nor does the card scroll sideways.
          const ink = await tab.evaluate((sel) => {
            const node = document.querySelector<HTMLElement>(sel)!;
            const style = getComputedStyle(node);
            const outer = node.getBoundingClientRect();
            const contentLeft = outer.left + node.clientLeft;
            const range = document.createRange();
            const lines: { left: number; right: number }[] = [];
            const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT);
            for (let text = walker.nextNode(); text; text = walker.nextNode()) {
              if (!text.textContent?.trim()) continue;
              range.selectNodeContents(text);
              for (const rect of range.getClientRects()) {
                if (rect.width > 0) lines.push({ left: rect.left, right: rect.right });
              }
            }
            return {
              innerLeft: contentLeft + parseFloat(style.paddingLeft),
              innerRight: contentLeft + node.clientWidth - parseFloat(style.paddingRight),
              left: Math.min(...lines.map((line) => line.left)),
              right: Math.max(...lines.map((line) => line.right)),
              scrollWidth: node.scrollWidth,
              clientWidth: node.clientWidth,
            };
          }, item.card);
          expect(ink.left, 'no text starts left of the card’s inner box').toBeGreaterThanOrEqual(
            ink.innerLeft - 0.5
          );
          expect(ink.right, 'no text runs past the card’s inner box').toBeLessThanOrEqual(
            ink.innerRight + 0.5
          );
          expect(ink.scrollWidth, 'the card does not scroll sideways').toBeLessThanOrEqual(
            ink.clientWidth
          );
          if (item === FOLDER) {
            // The long path wraps (so the check above measured a path that had to), and every one
            // of its lines ends inside the value that holds it, which does not scroll sideways.
            const path = await tab.evaluate(
              ({ sel, folder }) => {
                const value = [
                  ...document.querySelectorAll<HTMLElement>(`${sel} [data-path]`),
                ].find((node) => node.dataset.path === folder)!;
                const range = document.createRange();
                range.selectNodeContents(value);
                const lines = [...range.getClientRects()].filter((rect) => rect.width > 0);
                return {
                  lines: new Set(lines.map((rect) => Math.round(rect.top))).size,
                  right: Math.max(...lines.map((rect) => rect.right)),
                  boxRight: value.getBoundingClientRect().right,
                  scrollWidth: value.scrollWidth,
                  clientWidth: value.clientWidth,
                };
              },
              { sel: item.card, folder: LONG_PATH }
            );
            expect(path.lines, 'the long path takes more than one line').toBeGreaterThan(1);
            expect(path.right, 'each of its lines ends inside its value').toBeLessThanOrEqual(
              path.boxRight + 0.5
            );
            expect(path.scrollWidth, 'the value does not scroll sideways').toBeLessThanOrEqual(
              path.clientWidth
            );
          }
        } finally {
          await tab.close();
        }
      });

      it('stays open while the pointer travels up across the composer into the card, at a hand’s pace', async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load();
        try {
          const { onTrigger, card } = await hoverOpen(tab, item);
          // The way up crosses the composer and takes far longer than the close grace: the card
          // holds because the pointer keeps moving along its way to it.
          const inCard = {
            x: item.align === 'start' ? card.left + 40 : card.right - 40,
            y: card.bottom - 20,
          };
          const steps = 10;
          const pause = 40;
          expect(steps * pause).toBeGreaterThan(FOOTER_CARD_CLOSE_GRACE_MS * 2);
          await glide(tab, onTrigger, inCard, steps, pause);
          await tab.waitForTimeout(FOOTER_CARD_CLOSE_GRACE_MS * 3);
          expect(await cardState(tab, item), 'the pointer is in the card').toBe('open');

          // Away from both: it closes after the grace.
          await tab.mouse.move(450, 30, { steps: 6 });
          await waitClosed(tab, item);
        } finally {
          await tab.close();
        }
      });

      it('stays open while the pointer travels back down from the card across the composer to its trigger, at a hand’s pace', async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load();
        try {
          const { onTrigger, inCard } = await restInCard(tab, item);
          // Marked, so a card that closed on the way and that the arrival opened again after the
          // delay is told apart from one that never closed.
          await tab.evaluate(
            (sel) => document.querySelector(sel)!.setAttribute('data-probe', ''),
            item.card
          );
          // The way down crosses the composer and takes far longer than the close grace: the card
          // holds because the pointer keeps moving along its way to the trigger.
          const steps = 10;
          const pause = 40;
          expect(steps * pause).toBeGreaterThan(FOOTER_CARD_CLOSE_GRACE_MS * 2);
          for (let i = 1; i <= steps; i++) {
            await tab.mouse.move(
              inCard.x + ((onTrigger.x - inCard.x) * i) / steps,
              inCard.y + ((onTrigger.y - inCard.y) * i) / steps
            );
            await tab.waitForTimeout(pause);
            expect(await cardState(tab, item), `${i} of ${steps} moves down`).toBe('open');
          }
          // On its trigger, it stays for as long as the pointer does.
          await tab.waitForTimeout(FOOTER_CARD_CLOSE_GRACE_MS * 3);
          expect(await cardState(tab, item), 'the pointer is on the trigger').toBe('open');
          const same = await tab.evaluate(
            (sel) => document.querySelector(sel)?.hasAttribute('data-probe') ?? false,
            item.card
          );
          expect(same, 'the same card, never closed').toBe(true);
        } finally {
          await tab.close();
        }
      });

      it('closes for a pointer that leaves the card along the composer, away from its trigger, while it is still moving', async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load();
        try {
          const { composer, inCard } = await restInCard(tab, item);
          // Out of the card and sideways across the composer's text, away from the side the
          // trigger is on, never stopping: off the way to the trigger, a move re-arms nothing.
          const away = {
            x: item.align === 'start' ? composer.right - 40 : composer.left + 40,
            y: composer.bottom - 30,
          };
          const steps = 12;
          const pause = 40;
          expect(steps * pause).toBeGreaterThan(FOOTER_CARD_CLOSE_GRACE_MS * 3);
          await glide(tab, inCard, away, steps, pause);
          expect(await cardState(tab, item), 'closed while the pointer still moves').not.toBe(
            'open'
          );
        } finally {
          await tab.close();
        }
      });

      it('closes for a pointer that stops on the composer', async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load();
        try {
          const { onTrigger, composer } = await hoverOpen(tab, item);
          await glide(tab, onTrigger, { x: onTrigger.x, y: composer.bottom - 24 }, 3, 16);
          await tab.waitForTimeout(FOOTER_CARD_CLOSE_GRACE_MS * 3);
          expect(await cardState(tab, item)).not.toBe('open');
        } finally {
          await tab.close();
        }
      });

      it('closes for a pointer that heads off along the composer, away from the card, while it is still moving', async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load();
        try {
          const { onTrigger, composer } = await hoverOpen(tab, item);
          // Sideways across the composer's text, away from the side the card hangs on, and never
          // stopping: off the way to the card, a move re-arms nothing, so the grace runs out while
          // the pointer still moves. A hook that re-armed it on every move would hold the card open
          // over whatever the pointer crossed for as long as it kept moving.
          const away = {
            x: item.align === 'start' ? composer.right - 40 : composer.left + 40,
            y: composer.bottom - 30,
          };
          const steps = 12;
          const pause = 40;
          expect(steps * pause).toBeGreaterThan(FOOTER_CARD_CLOSE_GRACE_MS * 3);
          await glide(tab, onTrigger, away, steps, pause);
          expect(await cardState(tab, item), 'closed while the pointer still moves').not.toBe(
            'open'
          );
        } finally {
          await tab.close();
        }
      });

      it('takes focus into the card on a press inside, from a draft being typed in, with no focus edge and no selection', async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load();
        try {
          for (const press of item.presses) {
            await tab.click(DRAFT);
            await tab.keyboard.type('x');
            await hoverOpen(tab, item);
            const target = await box(tab, `${item.card} ${press}`);
            const at = {
              x: target.left + Math.min(6, target.width / 2),
              y: (target.top + target.bottom) / 2,
            };
            await tab.mouse.move(at.x, at.y, { steps: 12 });
            await tab.mouse.down();
            if (press.startsWith('.br-context-gauge')) {
              // The gauge's press starts a drag: across the card's text, it selects nothing.
              const label = await box(tab, `${item.card} .br-footer-card__label`);
              await tab.mouse.move(label.right - 2, (label.top + label.bottom) / 2, { steps: 6 });
            }
            await tab.mouse.up();
            const after = await tab.evaluate((sel) => {
              const active = document.activeElement!;
              return {
                inCard: document.querySelector(sel)!.contains(active),
                focusEdge: active.matches(':focus-visible'),
                selected: getSelection()?.toString() ?? '',
              };
            }, item.card);
            expect(after, press).toEqual({ inCard: true, focusEdge: false, selected: '' });
            expect(await cardState(tab, item), `${press}: pinned`).toBe('open');
            await tab.mouse.click(450, 30);
            await waitClosed(tab, item);
          }
        } finally {
          await tab.close();
        }
      });

      it('places the caret for a click into the draft right after a hover, instead of pinning the card', async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load();
        try {
          const { onTrigger } = await hoverOpen(tab, item);
          const draft = await box(tab, DRAFT);
          const target = {
            x: item.align === 'start' ? draft.left + 30 : draft.right - 30,
            y: draft.top + 20,
          };
          await glide(tab, onTrigger, target, 4, 10);
          await tab.mouse.down();
          await tab.mouse.up();
          await waitClosed(tab, item);
          expect(await activeTestId(tab)).toBe('draft');
        } finally {
          await tab.close();
        }
      });

      it('stays open once a press inside pins it, wherever the pointer goes, until an outside press', async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load();
        try {
          const { card } = await hoverOpen(tab, item);
          // Its section label: a press on the card's text, not on one of its controls.
          const label = await box(tab, `${item.card} .br-footer-card__label`);
          await tab.mouse.move(label.left + 4, (label.top + label.bottom) / 2, { steps: 12 });
          expect(card.top).toBeLessThanOrEqual(label.top);
          await tab.mouse.down();
          await tab.mouse.up();
          await tab.mouse.move(450, 30, { steps: 6 });
          await tab.waitForTimeout(FOOTER_CARD_CLOSE_GRACE_MS * 3);
          expect(await cardState(tab, item)).toBe('open');
          await tab.mouse.click(450, 30);
          await waitClosed(tab, item);
        } finally {
          await tab.close();
        }
      });

      it('does not open for a pointer that only passes over its trigger', async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load();
        try {
          const trigger = await box(tab, item.trigger);
          const y = (trigger.top + trigger.bottom) / 2;
          await tab.mouse.move(trigger.left - 30, y);
          await tab.mouse.move(trigger.right + 30, y, { steps: 4 });
          await tab.mouse.move(trigger.right + 30, y - 200, { steps: 2 });
          await tab.waitForTimeout(FOOTER_CARD_OPEN_DELAY_MS * 2);
          expect(await cardState(tab, item)).toBe('absent');
        } finally {
          await tab.close();
        }
      });

      it('stays above the composer and below the drag band in a short window', async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load({ width: 900, height: 330 });
        try {
          await tab.click(item.trigger);
          await waitOpen(tab, item);
          const card = await box(tab, item.card);
          const composer = await box(tab, COMPOSER);
          expect(card.top, 'clear of the window-drag band').toBeGreaterThanOrEqual(52 - 0.5);
          expect(card.bottom, 'never flipped over the draft').toBeLessThanOrEqual(
            composer.top + 0.5
          );
          // A card taller than the room scrolls inside it rather than spilling out of it: the
          // folder card, with its two grants, is.
          const overflow = await tab.evaluate((sel) => {
            const node = document.querySelector(sel)!;
            return {
              scrolls: node.scrollHeight > node.clientHeight,
              overflowY: getComputedStyle(node).overflowY,
            };
          }, item.card);
          expect(overflow.overflowY).toBe('auto');
          if (item === FOLDER) expect(overflow.scrolls, 'the card scrolls').toBe(true);
        } finally {
          await tab.close();
        }
      });

      it('draws the focus edge on the card when a keyboard pin moves focus onto it', async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load();
        try {
          await tab.focus(item.trigger);
          await tab.keyboard.press('Enter');
          await tab.waitForFunction(
            (sel) => document.activeElement === document.querySelector(sel),
            item.card,
            { timeout: 2000 }
          );
          const style = await tab.evaluate((sel) => {
            const computed = getComputedStyle(document.querySelector(sel)!);
            return { border: computed.borderTopColor, shadow: computed.boxShadow };
          }, item.card);
          expect(style.border).toBe('rgb(85, 85, 85)');
          expect(style.shadow, 'a 1px inset of the focus ink').toMatch(
            /rgb\(85, 85, 85\) 0px 0px 0px 1px inset/
          );
          expect(style.shadow.split('inset')[1] ?? '', 'the popover shadow stays').toMatch(/px/);
          if (item === FOLDER) await tab.waitForSelector(`${item.card} li`);
          // A Tab from the surface reaches the card's first control. A card that holds none closes
          // and the Tab carries on from its trigger to what follows it, rather than doing nothing.
          await tab.keyboard.press('Tab');
          const active = await tab.evaluate(() => ({
            role: document.activeElement?.getAttribute('role') ?? document.activeElement?.tagName,
            name:
              document.activeElement?.getAttribute('aria-label') ??
              document.activeElement?.textContent?.trim(),
          }));
          expect(active.role?.toLowerCase()).toBe(item.afterTab.role);
          expect(active.name).toBe(item.afterTab.name);
          if (item.afterTab.closes) await waitClosed(tab, item);
          else expect(await cardState(tab, item)).toBe('open');
        } finally {
          await tab.close();
        }
      });
    });
  }

  /**
   * How the folder card writes a folder (`FooterCardPath`): a reading like the card's others, `~`
   * for the home folder, each name isolated, breaking after a separator where it can. Each page
   * here says the home folder is known (the desktop's own daemon) and lists its own grants.
   */
  describe('the folder card’s folders', () => {
    const HOME = '/Users/someone';
    /** The grants listed and, when given, the chat's working folder in place of the default. */
    const setup = (grants: { id: string; folder: string }[], workingDir?: string) => `
      window.appConfig = {
        get: (key) => ({ BIOROUTER_HOME_DIR: ${JSON.stringify(HOME)}, BIOROUTER_LOCAL_BACKEND: true })[key],
      };
      window.__FOLDER_GRANTS__ = ${JSON.stringify(
        grants.map((grant) => ({ access: 'read-only', descendants: true, ...grant }))
      )};
      ${workingDir ? `window.__WORKING_DIR__ = ${JSON.stringify(workingDir)};` : ''}
    `;

    /** Open the folder card by a click, with the pointer off it, once its grants are listed. */
    async function openFolderCard(tab: Page) {
      await tab.click(FOLDER.trigger);
      await waitOpen(tab, FOLDER);
      await tab.mouse.move(450, 30);
    }
    /**
     * A folder as drawn: its text, its type beside the section label's, the type each of its
     * texts is drawn in, every line break (the characters either side of it, read from each
     * character's own box, so a break is where the drawing breaks whatever the markup says, and
     * the name it falls inside, if any), the names split across lines and the names wider than the
     * line by themselves (the only ones that may be), the drawn order of names and separators, what
     * a selection copies, and where its row's Revoke sits.
     */
    const drawn = (tab: Page, absolute: string) =>
      tab.evaluate(
        ({ sel, absolute }) => {
          const card = document.querySelector<HTMLElement>(sel)!;
          const value = [...card.querySelectorAll<HTMLElement>('[data-path]')].find(
            (node) => node.dataset.path === absolute
          );
          if (!value) return null;
          const names = [...value.querySelectorAll('bdi')];
          const texts: Text[] = [];
          const chars: { ch: string; name: Element | null; top: number; right: number }[] = [];
          const range = document.createRange();
          const walker = document.createTreeWalker(value, NodeFilter.SHOW_TEXT);
          for (let text = walker.nextNode(); text; text = walker.nextNode()) {
            texts.push(text as Text);
            const name = text.parentElement?.closest('bdi') ?? null;
            const content = text.textContent ?? '';
            for (let i = 0; i < content.length; i++) {
              range.setStart(text, i);
              range.setEnd(text, i + 1);
              const rect = range.getBoundingClientRect();
              chars.push({ ch: content[i], name, top: rect.top, right: rect.right });
            }
          }
          const breaks: { at: string; inside: string | null }[] = [];
          for (let i = 1; i < chars.length; i++) {
            if (chars[i].top <= chars[i - 1].top + 4) continue;
            const inside = chars[i].name !== null && chars[i].name === chars[i - 1].name;
            breaks.push({
              at: chars[i - 1].ch + chars[i].ch,
              inside: inside ? chars[i].name!.textContent : null,
            });
          }
          // The names drawn on more than one line: a name split inside itself.
          const split = names
            .filter((name) => {
              range.selectNodeContents(name);
              const tops = [...range.getClientRects()]
                .filter((rect) => rect.width > 0)
                .map((rect) => Math.round(rect.top));
              return new Set(tops).size > 1;
            })
            .map((name) => name.textContent);
          // The names that cannot fit a line however the path breaks: wider than the value on one
          // line, with the separators drawn beside them.
          const probe = document.createElement('span');
          probe.style.cssText = 'position: absolute; visibility: hidden; white-space: nowrap';
          value.appendChild(probe);
          const wide = names
            .filter((name) => {
              probe.textContent = (name.closest('.br-path-piece') ?? name).textContent;
              return probe.getBoundingClientRect().width > value.clientWidth + 0.5;
            })
            .map((name) => name.textContent);
          probe.remove();
          // What is drawn, in document order: each name (its `<bdi>`) and each separator (text
          // outside every `<bdi>`), whatever pieces group them. A name on one line comes after the
          // one before it, left to right on the same line or on a later line.
          const leaves: { text: string | null; left: number; right: number; top: number }[] = [];
          const nodes = document.createTreeWalker(
            value,
            NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT
          );
          for (let node = nodes.nextNode(); node; node = nodes.nextNode()) {
            const isName = node.nodeName === 'BDI';
            const isSeparator =
              node.nodeType === Node.TEXT_NODE && !node.parentElement?.closest('bdi');
            if (!isName && !isSeparator) continue;
            range.selectNodeContents(node);
            const rects = [...range.getClientRects()].filter((rect) => rect.width > 0);
            if (new Set(rects.map((rect) => Math.round(rect.top))).size !== 1) continue;
            const rect = range.getBoundingClientRect();
            leaves.push({
              text: node.textContent,
              left: rect.left,
              right: rect.right,
              top: rect.top,
            });
          }
          const misplaced: string[] = [];
          for (let i = 1; i < leaves.length; i++) {
            const [before, after] = [leaves[i - 1], leaves[i]];
            const sameLine = Math.abs(after.top - before.top) < 4;
            if (sameLine ? after.left < before.right - 0.5 : after.top < before.top)
              misplaced.push(`"${after.text}" drawn before "${before.text}"`);
          }
          // The type each text is drawn in, read from the element that holds it: every name's
          // `<bdi>`, and whatever holds the separators. A rule aimed at the names alone shows here,
          // where the paragraph's own computed style would not change.
          const inks = texts.map((text) => {
            const s = getComputedStyle(text.parentElement!);
            return {
              text: text.textContent,
              color: s.color,
              family: s.fontFamily,
              size: s.fontSize,
              lineHeight: s.lineHeight,
            };
          });
          // What a selection of the whole drawn path copies, line breaks and all.
          const selection = getSelection()!;
          const whole = document.createRange();
          whole.selectNodeContents(value);
          selection.removeAllRanges();
          selection.addRange(whole);
          const copied = selection.toString();
          selection.removeAllRanges();
          const type = (node: Element) => {
            const s = getComputedStyle(node);
            return { family: s.fontFamily, size: s.fontSize, lineHeight: s.lineHeight };
          };
          const box = value.getBoundingClientRect();
          const revoke = value.closest('li')?.querySelector('button')?.getBoundingClientRect();
          return {
            text: value.textContent,
            // What is drawn, as the page reads it: a space the layout drops is missing here,
            // where `textContent` still has it.
            innerText: value.innerText,
            type: type(value),
            labelType: type(card.querySelector('.br-footer-card__label')!),
            color: getComputedStyle(value).color,
            inks,
            breaks,
            split,
            wide,
            misplaced,
            copied,
            right: Math.max(...chars.map((c) => c.right)),
            boxRight: box.right,
            firstLineCentre: box.top + parseFloat(getComputedStyle(value).lineHeight) / 2,
            revokeCentre: revoke ? (revoke.top + revoke.bottom) / 2 : null,
          };
        },
        { sel: FOLDER.card, absolute }
      );

    it('writes a folder from `~` in the card’s own face, size and ink, and a selection copies what it says', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await load(undefined, setup([{ id: 'g1', folder: `${HOME}/data/results` }]));
      try {
        await openFolderCard(tab);
        const working = (await drawn(tab, `${HOME}/Projects/kdps`))!;
        expect(working.text).toBe('~/Projects/kdps');
        expect(working.type.family, 'the label’s face, not mono').toBe(working.labelType.family);
        expect(working.type.family).not.toMatch(/mono/i);
        expect(working.type).toMatchObject({ size: '13px', lineHeight: '18px' });
        expect(working.color).toBe('rgb(17, 17, 17)');
        expect(working.breaks, 'a short path stays on one line').toEqual([]);
        const grant = (await drawn(tab, `${HOME}/data/results`))!;
        expect(grant.text).toBe('~/data/results');
        // One ink for the whole path: every text in it, each name's `<bdi>` and the separators, in
        // `--text-default` and the label's face at 13/18, in the working folder and in a grant.
        for (const path of [working, grant]) {
          expect(path.inks.map((ink) => ink.text).join(''), 'every text is read').toBe(path.text);
          for (const ink of path.inks) {
            expect(ink, `"${ink.text}" in the path’s one ink and type`).toEqual({
              text: ink.text,
              color: 'rgb(17, 17, 17)',
              family: path.labelType.family,
              size: '13px',
              lineHeight: '18px',
            });
          }
        }
        const copied = await tab.evaluate((sel) => {
          const value = document.querySelector(`${sel} [data-path]`)!;
          const range = document.createRange();
          range.selectNodeContents(value);
          getSelection()!.removeAllRanges();
          getSelection()!.addRange(range);
          return getSelection()!.toString();
        }, FOLDER.card);
        expect(copied, 'nothing added, nothing lost').toBe('~/Projects/kdps');
      } finally {
        await tab.close();
      }
    });

    /**
     * Long folders, each drawn as the working folder (the card's full width) and as a grant
     * (narrower, beside its Revoke). A hyphen and a space are break points of ordinary text, so a
     * long path once broke inside a kebab-case or spaced name ("biorouter-" / "workflows-and-skills",
     * "single cell " / "atlas") where a break at the separator just before it would have fitted.
     * Every name here fits a line by itself, so each is kept whole and every break falls right
     * after a separator (a name wider than the line is the next test's).
     */
    const LONG_FOLDERS = [
      [
        'slashes',
        '/Volumes/shared/research/projects/alpha/beta/gamma/delta/epsilon/zeta/eta/theta',
      ],
      [
        'backslashes',
        'C:\\Users\\other\\Documents\\research\\projects\\alpha\\beta\\gamma\\delta\\epsilon\\zeta',
      ],
      [
        'names that start with a digit',
        '/Volumes/shared/2021/2022/2023/2024/2025/2026/0001/0002/0003/0004/0005',
      ],
      [
        'names with hyphens',
        `${HOME}/Developer/baranzini-lab/biorouter-workflows-and-skills/scripts`,
      ],
      [
        'hyphenated names on another volume',
        '/Volumes/lab-share/projects/multiple-sclerosis-progression-cohort/2026',
      ],
      [
        'names with spaces',
        `${HOME}/Documents/UCSF Baranzini Lab/single cell atlas/analysis notebooks`,
      ],
      [
        'right-to-left names',
        `${HOME}/מסמכים משותפים/نتائج التحليل النهائية/ניסוי-שני/analysis notebooks`,
      ],
    ] as const;
    const PLACES = ['the working folder', 'a grant row'] as const;

    it.for(
      LONG_FOLDERS.flatMap(([kind, folder]) =>
        PLACES.map((place) => [kind, place, folder] as const)
      )
    )('breaks a long path (%s) in %s only after a separator', async ([, place, folder], ctx) => {
      if (!browser) return ctx.skip();
      const inRow = place === 'a grant row';
      // A short grant beside the working folder, so the card has listed its grants.
      const tab = await load(
        undefined,
        inRow
          ? setup([{ id: 'g1', folder }])
          : setup([{ id: 'g1', folder: `${HOME}/data` }], folder)
      );
      try {
        await openFolderCard(tab);
        const path = (await drawn(tab, folder))!;
        expect(path.text).toBe(
          folder.startsWith(`${HOME}/`) ? `~${folder.slice(HOME.length)}` : folder
        );
        expect(path.wide, 'every name fits a line by itself').toEqual([]);
        expect(path.breaks.length, 'it needed more than one line').toBeGreaterThan(0);
        for (const { at } of path.breaks) expect(at, `a break at "${at}"`).toMatch(/^[\\/][^\\/]$/);
        expect(path.split, 'no name split inside itself').toEqual([]);
        expect(path.misplaced, 'names and separators in the order written').toEqual([]);
        expect(path.copied, 'a selection copies the path, nothing added').toBe(path.text);
        expect(path.right, 'every line ends inside the value').toBeLessThanOrEqual(
          path.boxRight + 0.5
        );
        if (inRow) {
          // However it wraps, its Revoke centres on the folder's first line.
          expect(path.revokeCentre, 'a grant row has its Revoke').not.toBeNull();
          expect(Math.abs(path.revokeCentre! - path.firstLineCentre)).toBeLessThanOrEqual(0.5);
        } else {
          expect(path.revokeCentre, 'the working folder has no Revoke').toBeNull();
        }
      } finally {
        await tab.close();
      }
    });

    it('breaks a name wider than the line at its own spaces, and keeps every other name whole', async (ctx) => {
      if (!browser) return ctx.skip();
      const wide = 'Notes on the renewal of the cohort grant for the next five years';
      const folder = `${HOME}/Library/Mobile Documents/${wide}/figures`;
      const tab = await load(undefined, setup([{ id: 'g1', folder }]));
      try {
        await openFolderCard(tab);
        const path = (await drawn(tab, folder))!;
        expect(path.text).toBe(`~/Library/Mobile Documents/${wide}/figures`);
        // Only the name that cannot fit a line breaks inside itself, and there at a space, as a
        // sentence would; "Mobile Documents" stays whole.
        expect(path.split).toEqual([wide]);
        expect(path.wide, 'the only name wider than the line').toEqual([wide]);
        expect(path.copied, 'a selection copies the path, nothing added').toBe(path.text);
        for (const { at } of path.breaks) expect(at, `a break at "${at}"`).toMatch(/^[ /]\S$/);
        expect(
          path.breaks.some(({ at, inside }) => inside === wide && at.startsWith(' ')),
          'the wide name breaks at a space'
        ).toBe(true);
        expect(path.right).toBeLessThanOrEqual(path.boxRight + 0.5);
      } finally {
        await tab.close();
      }
    });

    /**
     * A name that starts with a space begins its piece, and so a line of the piece's own (an
     * atomic box), where a collapsible space is dropped: "~/ Projects/kdps" was drawn and copied as
     * "~/Projects/kdps", another folder. A name of spaces alone lost all of them ("/tmp//x"), and
     * a run of spaces inside a name drew as one. `textContent` keeps every one of them, so only
     * what is drawn and what a selection copies show the loss.
     */
    it('keeps every space a folder’s names are written with, drawn and copied', async (ctx) => {
      if (!browser) return ctx.skip();
      const working = `${HOME}/ Projects/kdps`;
      const grants = [
        '/Users/me/ .ssh',
        '/Volumes/share/ Lab/data',
        '/tmp/ /x',
        `${HOME}/two  spaces/x`,
      ];
      const tab = await load(
        undefined,
        setup(
          grants.map((folder, index) => ({ id: `g${index + 1}`, folder })),
          working
        )
      );
      try {
        await openFolderCard(tab);
        for (const [folder, written] of [
          [working, '~/ Projects/kdps'],
          ['/Users/me/ .ssh', '/Users/me/ .ssh'],
          ['/Volumes/share/ Lab/data', '/Volumes/share/ Lab/data'],
          ['/tmp/ /x', '/tmp/ /x'],
          [`${HOME}/two  spaces/x`, '~/two  spaces/x'],
        ]) {
          const path = (await drawn(tab, folder))!;
          expect(path, `${folder} is listed`).not.toBeNull();
          expect(path.text).toBe(written);
          expect(path.innerText, `"${written}" drawn as written`).toBe(written);
          expect(path.copied, `"${written}" copied as written`).toBe(written);
          expect(path.misplaced, 'names and separators in the order written').toEqual([]);
        }
      } finally {
        await tab.close();
      }
    });

    it('keeps a right-to-left folder’s names in order, the separators between them', async (ctx) => {
      if (!browser) return ctx.skip();
      const folder = `${HOME}/מסמכים/نتائج/data`;
      const tab = await load(undefined, setup([{ id: 'g1', folder }]));
      try {
        await openFolderCard(tab);
        const pieces = await tab.evaluate(
          ({ sel, folder }) => {
            const value = [...document.querySelectorAll<HTMLElement>(`${sel} [data-path]`)].find(
              (node) => node.dataset.path === folder
            )!;
            const range = document.createRange();
            // What is drawn, in document order: each name (its `<bdi>`) and each separator (text
            // outside every `<bdi>`), whatever pieces group them.
            const leaves: Node[] = [];
            const walker = document.createTreeWalker(
              value,
              NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT
            );
            for (let node = walker.nextNode(); node; node = walker.nextNode()) {
              if (node.nodeName === 'BDI') leaves.push(node);
              else if (node.nodeType === Node.TEXT_NODE && !node.parentElement?.closest('bdi'))
                leaves.push(node);
            }
            return leaves.map((node) => {
              range.selectNodeContents(node);
              const rect = range.getBoundingClientRect();
              return {
                text: node.textContent,
                left: rect.left,
                right: rect.right,
                top: rect.top,
              };
            });
          },
          { sel: FOLDER.card, folder }
        );
        expect(pieces.map((piece) => piece.text)).toEqual([
          '~',
          '/',
          'מסמכים',
          '/',
          'نتائج',
          '/',
          'data',
        ]);
        expect(new Set(pieces.map((piece) => Math.round(piece.top))).size, 'one line').toBe(1);
        // Drawn left to right in the order written: no name trades places with a separator.
        for (let i = 1; i < pieces.length; i++) {
          expect(
            pieces[i].left,
            `${pieces[i].text} after ${pieces[i - 1].text}`
          ).toBeGreaterThanOrEqual(pieces[i - 1].right - 0.5);
        }
      } finally {
        await tab.close();
      }
    });

    /**
     * Chromium's triple-click selects a paragraph WITH the break after it (to the start of the next
     * block), so the copy of a triple-clicked folder read "~/data/results\n\n", and pasted after
     * `cd ` in a terminal it ran the line. A copy that starts in the path drops those breaks.
     */
    it('copies a triple-clicked folder without the paragraph break after it', async (ctx) => {
      if (!browser) return ctx.skip();
      const folder = `${HOME}/data/results`;
      const tab = await load(undefined, setup([{ id: 'g1', folder }]));
      try {
        await openFolderCard(tab);
        for (const path of [folder, `${HOME}/Projects/kdps`]) {
          const name = tab.locator(`${FOLDER.card} [data-path="${path}"] bdi`).last();
          await name.click({ clickCount: 3 });
          const copy = await tab.evaluate(() => {
            const selection = getSelection()!;
            const selected = selection.toString();
            // A copy, fired where Chromium fires it: at the element the selection starts in.
            const event = new ClipboardEvent('copy', {
              bubbles: true,
              cancelable: true,
              clipboardData: new DataTransfer(),
            });
            const start = selection.anchorNode!;
            (start instanceof Element ? start : start.parentElement)!.dispatchEvent(event);
            return {
              selected,
              copied: event.defaultPrevented
                ? event.clipboardData!.getData('text/plain')
                : selected,
            };
          });
          const written = `~${path.slice(HOME.length)}`;
          expect(copy.selected, 'the instrument: a triple-click selects the break too').toMatch(
            new RegExp(`^${written}\\n+$`)
          );
          expect(copy.copied, 'what the clipboard gets').toBe(written);
        }
      } finally {
        await tab.close();
      }
    });

    it('centres each Revoke on the first line of its folder, clear of the section label', async (ctx) => {
      if (!browser) return ctx.skip();
      // The default grants: a short folder and the long hash path, which takes several lines.
      const tab = await load();
      try {
        await openFolderCard(tab);
        const rows = await tab.evaluate((sel) => {
          const card = document.querySelector<HTMLElement>(sel)!;
          const list = card.querySelector('ul')!;
          const label = list.closest('section')!.querySelector('.br-footer-card__label')!;
          return {
            labelBottom: label.getBoundingClientRect().bottom,
            rows: [...list.querySelectorAll('li')].map((row) => {
              const value = row.querySelector<HTMLElement>('[data-path]')!;
              const button = row.querySelector('button')!.getBoundingClientRect();
              return {
                valueTop: value.getBoundingClientRect().top,
                valueHeight: value.getBoundingClientRect().height,
                lineHeight: parseFloat(getComputedStyle(value).lineHeight),
                buttonTop: button.top,
                buttonCentre: (button.top + button.bottom) / 2,
              };
            }),
          };
        }, FOLDER.card);
        expect(rows.rows).toHaveLength(2);
        expect(rows.rows[1].valueHeight, 'the second folder wraps').toBeGreaterThan(
          rows.rows[1].lineHeight * 1.5
        );
        for (const row of rows.rows) {
          expect(
            Math.abs(row.buttonCentre - (row.valueTop + row.lineHeight / 2))
          ).toBeLessThanOrEqual(0.5);
        }
        expect(rows.rows[0].buttonTop, 'Revoke clears the label above it').toBeGreaterThanOrEqual(
          rows.labelBottom
        );
      } finally {
        await tab.close();
      }
    });
  });

  it('dresses the three cards alike: one surface, one type for the same kind of line', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      const looks: Record<string, unknown>[] = [];
      for (const item of ITEMS) {
        await tab.click(item.trigger);
        await waitOpen(tab, item);
        // Off the trigger, so what is measured is the pinned card's own look.
        await tab.mouse.move(450, 30);
        looks.push(
          await tab.evaluate((sel) => {
            const card = document.querySelector<HTMLElement>(sel)!;
            const look = (node: Element | null) => {
              if (!node) return null;
              const s = getComputedStyle(node);
              return {
                fontSize: s.fontSize,
                lineHeight: s.lineHeight,
                color: s.color,
                fontWeight: s.fontWeight,
              };
            };
            const s = getComputedStyle(card);
            const sections = [...card.querySelectorAll('.br-footer-card__section')];
            return {
              width: card.getBoundingClientRect().width,
              padding: [s.paddingTop, s.paddingRight, s.paddingBottom, s.paddingLeft],
              radius: [
                s.borderTopLeftRadius,
                s.borderTopRightRadius,
                s.borderBottomRightRadius,
                s.borderBottomLeftRadius,
              ],
              background: s.backgroundColor,
              border: [s.borderTopWidth, s.borderTopStyle, s.borderTopColor, s.borderLeftColor],
              shadow: s.boxShadow,
              gap: s.rowGap,
              sectionGap: getComputedStyle(sections[0]).rowGap,
              label: look(card.querySelector('.br-footer-card__label')),
              value: look(card.querySelector('.br-footer-card__value')),
              detail: look(card.querySelector('.br-footer-card__detail')),
            };
          }, item.card)
        );
        await tab.mouse.click(450, 30);
        await waitClosed(tab, item);
      }
      const [folder, context, cost] = looks;
      // Not a likeness of three blank boxes: the surface carries its recipe.
      expect(folder.width).toBeCloseTo(320, 0);
      expect(folder.padding).toEqual(['12px', '12px', '12px', '12px']);
      expect(folder.radius).toEqual(['12px', '12px', '12px', '12px']);
      expect(folder.background).toBe('rgb(255, 255, 255)');
      expect(folder.border).toEqual(['1px', 'solid', 'rgb(204, 204, 204)', 'rgb(204, 204, 204)']);
      expect(folder.shadow).toMatch(/px/);
      expect(folder.gap).toBe('12px');
      expect(folder.label).toMatchObject({ color: 'rgb(102, 102, 102)' });
      expect(folder.value).toMatchObject({ color: 'rgb(17, 17, 17)' });
      expect(folder.detail).toMatchObject({ color: 'rgb(102, 102, 102)' });
      expect(context, 'the context card wears the folder card’s look').toEqual(folder);
      expect(cost, 'the cost card wears the folder card’s look').toEqual(folder);
    } finally {
      await tab.close();
    }
  });

  /**
   * A card's buttons (Choose folder…, Revoke, Compact chat) read at the card's reading size, the
   * 13/18 of the values beside them. The shared xs button sets the 14/20 control role, which made
   * a button's label the largest text in a card of 13px readings and 12px labels.
   */
  it('sets a card’s buttons at the reading size of the values beside them', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      for (const item of [FOLDER, CONTEXT]) {
        await tab.click(item.trigger);
        await waitOpen(tab, item);
        await tab.mouse.move(450, 30);
        const type = await tab.evaluate((sel) => {
          const card = document.querySelector<HTMLElement>(sel)!;
          const look = (node: Element) => {
            const s = getComputedStyle(node);
            return { size: s.fontSize, lineHeight: s.lineHeight, weight: s.fontWeight };
          };
          return {
            value: look(card.querySelector('.br-footer-card__value')!),
            buttons: [...card.querySelectorAll('button[data-slot="button"]')].map((button) => ({
              text: button.textContent,
              ...look(button),
            })),
          };
        }, item.card);
        expect(type.buttons.length, `the ${item.name} card has a button`).toBeGreaterThan(0);
        for (const button of type.buttons) {
          expect(button, `"${button.text}" at the value’s size`).toEqual({
            text: button.text,
            size: type.value.size,
            lineHeight: type.value.lineHeight,
            // The control role's weight stays: it is a button.
            weight: '500',
          });
        }
        await tab.mouse.click(450, 30);
        await waitClosed(tab, item);
      }
    } finally {
      await tab.close();
    }
  });

  /** A trigger's paint and the two attributes that say what its card is doing. */
  const paint = (tab: Page, item: Item) =>
    tab.evaluate((sel) => {
      const node = document.querySelector(sel)!;
      const s = getComputedStyle(node);
      return {
        background: s.backgroundColor,
        color: s.color,
        expanded: node.getAttribute('aria-expanded'),
        pinned: node.hasAttribute('data-pinned'),
      };
    }, item.trigger);

  /** Somewhere on the page that is none of the triggers, the cards or the composer. */
  const AWAY = { x: 450, y: 30 };

  /**
   * HOVER PEEKS, CLICK PINS, as each trigger draws it. Only a click leaves a mark: the pointer on
   * a trigger washes it, a preview holds nothing (with the pointer in the card, the trigger is at
   * rest), and a KEPT card holds the strong fill on its trigger, with the pointer off it and back
   * on it. Kept is a subtle step from hover (about 1.09:1); what it must be is different.
   */
  it('draws rest, the pointer on it, a preview and a kept card the same way on all three triggers', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      const ladders = [];
      for (const item of ITEMS) {
        await tab.mouse.move(AWAY.x, AWAY.y);
        const rest = await paint(tab, item);
        const onTrigger = centre(await box(tab, item.trigger));
        await tab.mouse.move(onTrigger.x, onTrigger.y);
        const pointerOn = await paint(tab, item);
        await waitOpen(tab, item);
        const pointerOnOpen = await paint(tab, item);
        // Up into the card, at a hand's pace, and resting there: the trigger holds nothing.
        await tab.mouse.move(AWAY.x, AWAY.y);
        await waitClosed(tab, item);
        await restInCard(tab, item);
        const preview = await paint(tab, item);
        await tab.mouse.move(AWAY.x, AWAY.y, { steps: 6 });
        await waitClosed(tab, item);
        // Kept by a click: the pointer off it, then back on it.
        await tab.click(item.trigger);
        await waitOpen(tab, item);
        await tab.mouse.move(AWAY.x, AWAY.y);
        const kept = await paint(tab, item);
        await tab.mouse.move(onTrigger.x, onTrigger.y);
        const keptPointerOn = await paint(tab, item);
        await tab.mouse.click(AWAY.x, AWAY.y);
        await waitClosed(tab, item);
        const closed = await paint(tab, item);
        ladders.push({ rest, pointerOn, pointerOnOpen, preview, kept, keptPointerOn, closed });
      }
      const [folder, context, cost] = ladders;
      expect(folder.rest).toEqual({
        background: 'rgba(0, 0, 0, 0)',
        color: 'rgb(102, 102, 102)',
        expanded: 'false',
        pinned: false,
      });
      expect(folder.pointerOn, 'the pointer on it').toEqual({
        background: 'rgba(0, 0, 0, 0.05)',
        color: 'rgb(17, 17, 17)',
        expanded: 'false',
        pinned: false,
      });
      expect(folder.pointerOnOpen, 'the same wash once its card shows').toEqual({
        ...folder.pointerOn,
        expanded: 'true',
      });
      expect(folder.preview, 'a preview, the pointer in the card: the rest look').toEqual({
        ...folder.rest,
        expanded: 'true',
      });
      expect(folder.kept, 'kept, the pointer off it').toEqual({
        background: 'rgb(234, 234, 230)',
        color: 'rgb(17, 17, 17)',
        expanded: 'true',
        pinned: true,
      });
      expect(folder.keptPointerOn, 'kept, the pointer back on it: still kept').toEqual(folder.kept);
      expect(folder.kept.background, 'kept is not the hover wash').not.toBe(
        folder.pointerOn.background
      );
      expect(folder.closed, 'closed: at rest again').toEqual(folder.rest);
      expect(context, 'the ring draws the folder chip’s states').toEqual(folder);
      expect(cost, 'the cost draws the folder chip’s states').toEqual(folder);
    } finally {
      await tab.close();
    }
  });

  it('edges a kept trigger in forced colours, where every fill is repainted, and nothing else', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      await tab.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
      const edge = (item: Item) =>
        tab.evaluate((sel) => {
          const s = getComputedStyle(document.querySelector(sel)!);
          return {
            style: s.outlineStyle,
            width: s.outlineWidth,
            offset: s.outlineOffset,
            color: s.outlineColor,
          };
        }, item.trigger);
      for (const item of ITEMS) {
        await tab.mouse.move(AWAY.x, AWAY.y);
        expect((await edge(item)).style, `${item.name} at rest`).toBe('none');
        const onTrigger = centre(await box(tab, item.trigger));
        await tab.mouse.move(onTrigger.x, onTrigger.y);
        await waitOpen(tab, item);
        expect((await edge(item)).style, `${item.name} previewing`).toBe('none');
        await tab.mouse.move(AWAY.x, AWAY.y, { steps: 6 });
        await waitClosed(tab, item);
        await tab.click(item.trigger);
        await waitOpen(tab, item);
        await tab.mouse.move(AWAY.x, AWAY.y);
        const kept = await edge(item);
        expect(kept, `${item.name} kept`).toMatchObject({
          style: 'solid',
          width: '1px',
          offset: '-1px',
        });
        // A system colour, so forced colours keep it: drawn, not transparent.
        expect(kept.color, `${item.name} kept edge colour`).toMatch(/^rgb\(/);
        await tab.mouse.click(AWAY.x, AWAY.y);
        await waitClosed(tab, item);
        expect((await edge(item)).style, `${item.name} closed`).toBe('none');
      }
    } finally {
      await tab.close();
    }
  });

  it('keeps the context ring’s circle when its card is kept, on the strong fill', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      const ring = () =>
        tab.evaluate((sel) => {
          const trigger = document.querySelector(sel)!;
          const stroke = (selector: string) =>
            getComputedStyle(trigger.querySelector(selector)!).stroke;
          return {
            ground: getComputedStyle(trigger).backgroundColor,
            track: stroke('[data-testid="context-ring-track"]'),
            arc: stroke('[data-testid="context-ring-arc"]'),
          };
        }, CONTEXT.trigger);
      const rgb = (css: string) => {
        const [r, g, b, a = 1] = css.match(/[\d.]+/g)!.map(Number);
        // A transparent ground is the page's: white here.
        return a === 0 ? [255, 255, 255] : [r, g, b];
      };
      const luminance = (css: string) => {
        const [r, g, b] = rgb(css).map((v) => {
          const c = v / 255;
          return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
        });
        return 0.2126 * r + 0.7152 * g + 0.0722 * b;
      };
      const contrast = (a: string, b: string) => {
        const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
        return (hi + 0.05) / (lo + 0.05);
      };

      await tab.mouse.move(AWAY.x, AWAY.y);
      const rest = await ring();
      await tab.click(CONTEXT.trigger);
      await waitOpen(tab, CONTEXT);
      await tab.mouse.move(AWAY.x, AWAY.y);
      const kept = await ring();
      expect(kept.ground, 'kept: the strong fill').toBe('rgb(234, 234, 230)');
      const atRest = contrast(rest.track, rest.ground);
      const keptStep = contrast(kept.track, kept.ground);
      // What the rule is for: the resting track's ink all but vanishes on the kept fill.
      expect(contrast(rest.track, kept.ground), 'the resting ink on the kept fill').toBeLessThan(
        1.1
      );
      // So a kept ring's track is redrawn: a step off the fill, as the resting one is off the page.
      expect(kept.track, 'a deeper track ink when kept').not.toBe(rest.track);
      expect(keptStep, 'the kept track is a visible step off the fill').toBeGreaterThanOrEqual(
        1.18
      );
      expect(keptStep, 'about the step it has at rest').toBeGreaterThanOrEqual(atRest * 0.95);
      expect(kept.arc, 'the arc in the default ink').toBe('rgb(17, 17, 17)');
    } finally {
      await tab.close();
    }
  });

  it('keeps one card open at a time: a hover opens nothing over a kept card, a click replaces it, a hover replaces a preview', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      const open = async () => {
        const states = await Promise.all(ITEMS.map((item) => cardState(tab, item)));
        return ITEMS.filter((_, index) => states[index] === 'open').map((item) => item.name);
      };
      await tab.click(FOLDER.trigger);
      await waitOpen(tab, FOLDER);
      // A hover on the ring, resting there well past its delay: nothing opens over the kept
      // folder card, which stays, and so does the focus the chip's click gave it.
      const ring = centre(await box(tab, CONTEXT.trigger));
      await tab.mouse.move(ring.x, ring.y, { steps: 4 });
      await tab.waitForTimeout(FOOTER_CARD_OPEN_DELAY_MS * 3);
      expect(await open()).toEqual(['folder']);
      expect(await activeTestId(tab)).toBe('dir-switcher-locked');
      // A click on the ring: the folder card goes.
      await tab.click(CONTEXT.trigger);
      await waitOpen(tab, CONTEXT);
      await waitClosed(tab, FOLDER);
      expect(await open()).toEqual(['context']);
      // A click on the cost: the ring's card goes.
      await tab.click(COST.trigger);
      await waitOpen(tab, COST);
      await waitClosed(tab, CONTEXT);
      expect(await open()).toEqual(['cost']);
      expect(await activeTestId(tab)).toBe('chat-cost');
      // An outside press closes it. The pointer leaves for the page first, as a hand does: a press
      // there in the same moment as the click that opened the card can land before Radix listens
      // for an outside press (measured: the card then stayed open now and then, the cost's alone
      // on the line included, so it is not the kept rule).
      await tab.mouse.move(AWAY.x, AWAY.y, { steps: 4 });
      await tab.mouse.click(AWAY.x, AWAY.y);
      await waitClosed(tab, COST);
      // Then a hover preview of the ring: a hover on the folder chip replaces a preview.
      await tab.mouse.move(ring.x, ring.y, { steps: 4 });
      await waitOpen(tab, CONTEXT);
      const folder = centre(await box(tab, FOLDER.trigger));
      await tab.mouse.move(folder.x, folder.y, { steps: 4 });
      await waitOpen(tab, FOLDER);
      await waitClosed(tab, CONTEXT);
      expect(await open()).toEqual(['folder']);
    } finally {
      await tab.close();
    }
  });
});
