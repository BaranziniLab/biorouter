// @vitest-environment node
/**
 * The footer cards under a real pointer and a real layout, in Chromium, for ALL THREE items on the
 * line (the working folder, the context ring and the cost):
 *   - each card opens on a hover after the delay, and a pointer that only passes over its trigger
 *     opens nothing;
 *   - each hangs ABOVE THE WHOLE COMPOSER card, never over the draft, `+` or Tools, nor over the
 *     trigger it came from, lined up with the folder chip's left edge or with a readout's right
 *     edge, and below the window-drag band even in a short window;
 *   - the pointer can travel from any trigger up across the composer into its card without losing
 *     it, at a hand's pace; a pointer that stops on the composer or heads elsewhere closes it; a
 *     click into the draft right after a hover places the caret rather than pinning the card;
 *   - the three cards wear ONE surface: the same width, padding, radius, ground, hairline and
 *     shadow, and the same label, value and detail type; the three triggers show an open card the
 *     same way; and a card holding keyboard focus shows it, each the same;
 *   - only one card is open at a time.
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
  '--dur-fast-min',
  '--dur-med-min',
  '--ease-out',
  '--shadow-popover',
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
`;

const GRANTS = [
  { id: 'grant-1', folder: '/Users/someone/data/results', access: 'read-only', descendants: true },
  {
    id: 'grant-2',
    folder: '/Volumes/shared/an/unusually/long/path/that/has/to/wrap/inside/the/card/notes',
    access: 'read/write',
    descendants: false,
  },
];

/**
 * Everything the line needs that is not UI, keyed by the module's path under `src/`, so a stub
 * replaces it whichever file imports it and however that file spells the path.
 */
const STUBS: Record<string, string> = {
  api: `
    export const listFolderAccess = async () => ({ data: ${JSON.stringify(GRANTS)} });
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
 * (the composer card handed over as `composerRef`). A chat with messages, a known folder, a
 * loaded window with tokens and a cost above $0, so all three items are on the line.
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
        workingDir="/Users/someone/Projects/kdps"
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
      PICKERS_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function load(viewport = { width: 900, height: 720 }): Promise<Page> {
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
          // Nothing in it is wider than the card (a long path wraps inside it).
          const widest = await tab.evaluate(
            (sel) =>
              Math.max(
                ...[...document.querySelectorAll(`${sel} *`)].map(
                  (node) => node.getBoundingClientRect().right
                )
              ),
            item.card
          );
          expect(widest).toBeLessThanOrEqual(card.right + 0.5);
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

  it('shows an open card on each trigger the same way, and the same resting look', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      const paint = (item: Item) =>
        tab.evaluate((sel) => {
          const s = getComputedStyle(document.querySelector(sel)!);
          return { background: s.backgroundColor, color: s.color, radius: s.borderTopLeftRadius };
        }, item.trigger);
      await tab.mouse.move(450, 30);
      const resting = await Promise.all(ITEMS.map(paint));
      const open = [];
      for (const item of ITEMS) {
        await tab.click(item.trigger);
        await waitOpen(tab, item);
        await tab.mouse.move(450, 30);
        open.push(await paint(item));
        await tab.mouse.click(450, 30);
        await waitClosed(tab, item);
      }
      expect(resting[1]).toEqual(resting[0]);
      expect(resting[2]).toEqual(resting[0]);
      expect(open[1]).toEqual(open[0]);
      expect(open[2]).toEqual(open[0]);
      expect(open[0], 'an open card shows on its trigger').not.toEqual(resting[0]);
      expect(open[0]).toMatchObject({
        background: 'rgba(0, 0, 0, 0.05)',
        color: 'rgb(17, 17, 17)',
      });
    } finally {
      await tab.close();
    }
  });

  it('keeps one card open at a time: a hover or a click on another item closes the open one', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      const open = async () => {
        const states = await Promise.all(ITEMS.map((item) => cardState(tab, item)));
        return ITEMS.filter((_, index) => states[index] === 'open').map((item) => item.name);
      };
      await tab.click(FOLDER.trigger);
      await waitOpen(tab, FOLDER);
      // A hover on the ring: the folder card goes, and the focus the chip's click gave it stays.
      const ring = centre(await box(tab, CONTEXT.trigger));
      await tab.mouse.move(ring.x, ring.y, { steps: 4 });
      await waitOpen(tab, CONTEXT);
      await waitClosed(tab, FOLDER);
      expect(await open()).toEqual(['context']);
      expect(await activeTestId(tab)).toBe('dir-switcher-locked');
      // A click on the cost: the ring's card goes.
      await tab.click(COST.trigger);
      await waitOpen(tab, COST);
      await waitClosed(tab, CONTEXT);
      expect(await open()).toEqual(['cost']);
      expect(await activeTestId(tab)).toBe('chat-cost');
      // And back to the folder by a click.
      await tab.click(FOLDER.trigger);
      await waitOpen(tab, FOLDER);
      await waitClosed(tab, COST);
      expect(await open()).toEqual(['folder']);
    } finally {
      await tab.close();
    }
  });
});
