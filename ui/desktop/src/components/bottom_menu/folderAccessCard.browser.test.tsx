// @vitest-environment node
/**
 * The folder chip's card under a real pointer, in Chromium: hovering the chip opens it after
 * the delay; the card hangs ABOVE THE WHOLE COMPOSER card, never over the draft, `+` or Tools,
 * nor over the chip it came from; the pointer can travel from the chip up across the composer
 * into the card without losing it, at a hand's pace; a pointer that stops on the composer or
 * heads elsewhere closes it; and a click into the draft right after a hover places the caret
 * rather than landing on (and pinning) the card.
 *
 * jsdom cannot say any of this: it has no pointer, dispatches no enter or leave as a pointer
 * crosses a gap, and places nothing, so Radix's positioning and the authored geometry in
 * `pickers.css` never meet. So the REAL `DirSwitcher`, `useFolderCard`, `FolderAccessCard` and
 * `Popover` are bundled with esbuild and rendered in Chromium. Only the daemon's API, the toasts
 * and the proof-of-user headers are stand-ins.
 *
 * ⚠ Tailwind does not run here, so a utility's colour or radius is absent. Nothing below depends
 * on one: the card's width and padding are authored CSS (`.br-folder-card`), and every
 * assertion compares the card with the chip or with itself.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { FOLDER_CARD_CLOSE_GRACE_MS, FOLDER_CARD_OPEN_DELAY_MS } from './useFolderCard';

const here = dirname(fileURLToPath(import.meta.url));
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
  '--ease-out',
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

/** Tailwind's preflight, the parts the layout depends on, plus the floating surface's paint. */
const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
button, input { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
p { margin: 0; }
svg { display: block; width: 14px; height: 14px; }
.block { display: block; }
.font-mono { font-family: monospace; }
:root { --text-muted: #666; --text-default: #111; --overlay-hover: rgb(0 0 0 / 0.05);
  --background-focus: #ddd; --border-focus: #555; --border-subtle: #ccc; }
.biorouter-popover-surface { background: #fff; border: 1px solid #ccc; }
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

/** The daemon, the toasts and the proof headers: everything the chip needs that is not UI. */
const STUBS: Record<string, string> = {
  '../../api': `
    export const listFolderAccess = async () => ({ data: ${JSON.stringify(GRANTS)} });
    export const revokeFolderAccess = async () => ({ data: { revoked: true } });
    export const updateWorkingDir = async () => ({ data: {} });
  `,
  '../../toasts': 'export const toastError = () => {};',
  '../../utils/userAction': 'export const userActionHeaders = async () => ({});',
};

const stubDaemon: Plugin = {
  name: 'stub-daemon',
  setup(builder) {
    builder.onResolve({ filter: /^\.\.\/\.\.\/(api|toasts|utils\/userAction)$/ }, (args) => ({
      path: args.path,
      namespace: 'daemon-stub',
    }));
    builder.onLoad({ filter: /.*/, namespace: 'daemon-stub' }, (args) => ({
      contents: STUBS[args.path],
      loader: 'ts',
      resolveDir: here,
    }));
  },
};

/**
 * A chat's composer stand-in near the window's bottom edge: the composer card holding a draft,
 * and under it the footer line with the folder chip, as `ChatInput` lays them out (a column with
 * a 4px gap) and wires them (`composerRef`).
 */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { DirSwitcher } from './DirSwitcher';

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
      <div className="br-footline">
        <div className="br-footline__side">
          <DirSwitcher
            sessionId="chat-1"
            workingDir="/Users/someone/Projects/kdps"
            locked
            composerRef={composer}
          />
        </div>
      </div>
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

const CHIP = '[data-testid="dir-switcher-locked"]';
const CARD = '[data-testid="folder-access-card"]';
const COMPOSER = '[data-testid="composer"]';
const DRAFT = '[data-testid="draft"]';

describe('the folder card under a real pointer', () => {
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
    css = PREFLIGHT + readTokens() + PICKERS_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function load(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 720 } });
    // The open keyframe moves the surface; measure the layout, not the motion.
    await tab.emulateMedia({ reducedMotion: 'reduce' });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector(CHIP);
    return tab;
  }

  const box = (tab: Page, selector: string) =>
    tab.evaluate((sel) => {
      const r = document.querySelector(sel)!.getBoundingClientRect();
      return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, width: r.width };
    }, selector) as Promise<Box>;

  const cardState = (tab: Page) =>
    tab.evaluate(
      (sel) => document.querySelector(sel)?.getAttribute('data-state') ?? 'absent',
      CARD
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

  async function hoverOpen(tab: Page) {
    const chip = await box(tab, CHIP);
    const onChip = centre(chip);
    await tab.mouse.move(onChip.x, onChip.y);
    await tab.waitForSelector(`${CARD}[data-state="open"]`, { timeout: 2000 });
    await tab.waitForSelector(`${CARD} li`);
    return { chip, onChip, card: await box(tab, CARD), composer: await box(tab, COMPOSER) };
  }

  const waitClosed = (tab: Page) =>
    tab.waitForFunction(
      (sel) => document.querySelector(sel)?.getAttribute('data-state') !== 'open',
      CARD,
      { timeout: 2000 }
    );

  it('opens on hover and hangs above the whole composer, covering neither the draft nor the chip', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      const chip = await box(tab, CHIP);
      const onChip = centre(chip);
      await tab.mouse.move(onChip.x, onChip.y);

      // Not before the delay...
      await tab.waitForTimeout(FOLDER_CARD_OPEN_DELAY_MS / 2);
      expect(await cardState(tab)).toBe('absent');
      // ...and open after it, with the grants read and listed.
      await tab.waitForSelector(`${CARD}[data-state="open"]`, { timeout: 2000 });
      await tab.waitForSelector(`${CARD} li`);
      expect(await tab.locator(`${CARD} li`).count()).toBe(GRANTS.length);

      // Above the composer card, never over it, and on the chip's start edge.
      const card = await box(tab, CARD);
      const composer = await box(tab, COMPOSER);
      expect(card.bottom, 'the card ends above the composer').toBeLessThanOrEqual(
        composer.top + 0.5
      );
      expect(composer.top - card.bottom, 'a small gap, not a jump').toBeLessThanOrEqual(12);
      expect(Math.abs(card.left - chip.left), 'aligned to the chip').toBeLessThanOrEqual(1);
      expect(card.width, 'the authored width').toBeCloseTo(320, 0);
      // What is under the chip and under the draft is still the chip and the draft.
      const under = (point: { x: number; y: number }) =>
        tab.evaluate(
          ({ x, y }) =>
            document.elementFromPoint(x, y)?.closest('[data-testid]')?.getAttribute('data-testid'),
          point
        );
      expect(await under(onChip)).toBe('dir-switcher-locked');
      const draft = await box(tab, DRAFT);
      expect(await under({ x: draft.left + 20, y: draft.top + 10 })).toBe('draft');
      expect(await under(centre(draft))).toBe('draft');
      // A long path wraps inside the card instead of widening it.
      const widest = await tab.evaluate(
        (sel) =>
          Math.max(
            ...[...document.querySelectorAll(`${sel} .br-folder-card__path`)].map(
              (node) => node.getBoundingClientRect().right
            )
          ),
        CARD
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
      const { onChip, card } = await hoverOpen(tab);
      // The way up crosses the composer and takes far longer than the close grace: the card
      // holds because the pointer keeps moving along its way to it.
      const inCard = { x: card.left + 40, y: card.bottom - 20 };
      const steps = 10;
      const pause = 40;
      expect(steps * pause).toBeGreaterThan(FOLDER_CARD_CLOSE_GRACE_MS * 2);
      await glide(tab, onChip, inCard, steps, pause);
      await tab.waitForTimeout(FOLDER_CARD_CLOSE_GRACE_MS * 3);
      expect(await cardState(tab), 'the pointer is in the card').toBe('open');

      // Away from both: it closes after the grace.
      await tab.mouse.move(860, 40, { steps: 6 });
      await waitClosed(tab);
    } finally {
      await tab.close();
    }
  });

  it('closes for a pointer that stops on the composer, or heads along it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      // Stops on the way: the grace runs out.
      const first = await hoverOpen(tab);
      await glide(tab, first.onChip, { x: first.onChip.x, y: first.composer.bottom - 24 }, 3, 16);
      await tab.waitForTimeout(FOLDER_CARD_CLOSE_GRACE_MS * 3);
      expect(await cardState(tab)).not.toBe('open');

      // Heads off along the composer's text, away from the card, and keeps moving: it is closed
      // well before the pointer stops, because a pointer off the way to the card re-arms nothing.
      await tab.mouse.move(860, 700);
      const second = await hoverOpen(tab);
      const steps = 12;
      const pause = 40;
      expect(steps * pause).toBeGreaterThan(FOLDER_CARD_CLOSE_GRACE_MS * 3);
      await glide(
        tab,
        second.onChip,
        { x: second.composer.right - 40, y: second.composer.bottom - 30 },
        steps,
        pause
      );
      expect(await cardState(tab), 'closed while the pointer still moves').not.toBe('open');
    } finally {
      await tab.close();
    }
  });

  it('places the caret for a click into the draft right after a hover, instead of pinning the card', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      const { onChip } = await hoverOpen(tab);
      const draft = await box(tab, DRAFT);
      const target = { x: draft.left + 30, y: draft.top + 20 };
      await glide(tab, onChip, target, 4, 10);
      await tab.mouse.down();
      await tab.mouse.up();
      await waitClosed(tab);
      expect(await tab.evaluate(() => document.activeElement?.getAttribute('data-testid'))).toBe(
        'draft'
      );
    } finally {
      await tab.close();
    }
  });

  it('stays open once a press inside pins it, wherever the pointer goes', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      const onChip = centre(await box(tab, CHIP));
      await tab.mouse.move(onChip.x, onChip.y);
      await tab.waitForSelector(`${CARD}[data-state="open"]`, { timeout: 2000 });
      const card = await box(tab, CARD);
      await tab.mouse.move(card.left + 40, card.top + 16, { steps: 12 });
      await tab.mouse.down();
      await tab.mouse.up();
      await tab.mouse.move(860, 40, { steps: 6 });
      await tab.waitForTimeout(FOLDER_CARD_CLOSE_GRACE_MS * 3);
      expect(await cardState(tab)).toBe('open');
      // An outside press still closes it.
      await tab.mouse.click(860, 40);
      await tab.waitForFunction(
        (sel) => document.querySelector(sel)?.getAttribute('data-state') !== 'open',
        CARD,
        { timeout: 2000 }
      );
    } finally {
      await tab.close();
    }
  });

  it('does not open for a pointer that only passes over the chip', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      const chip = await box(tab, CHIP);
      await tab.mouse.move(chip.left - 30, chip.top + 4);
      await tab.mouse.move(chip.right + 60, chip.top + 4, { steps: 4 });
      await tab.waitForTimeout(FOLDER_CARD_OPEN_DELAY_MS * 2);
      expect(await cardState(tab)).toBe('absent');
    } finally {
      await tab.close();
    }
  });
});
