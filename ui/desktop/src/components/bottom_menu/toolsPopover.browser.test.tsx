// @vitest-environment node
/**
 * R-25 (owner message 13): the Tools popover's segmented control sits on the
 * popover's bottom edge, directly above the chip, and stays at the same place
 * whatever the list on screen and whatever the window's height, so switching
 * from Extensions to Skills to Knowledge never moves the control under the
 * pointer that clicked it.
 *
 * Only a layout engine can say that: jsdom places nothing, and the claim is
 * about Radix's positioning (a `side="top"` popover is pinned by its bottom
 * edge, collisions off) meeting `pickers.css` (the cap from the room above).
 * So the REAL `ToolsChip`, `Popover`, `SegmentedControl` and `PickerPanel` are
 * bundled with esbuild and rendered in Chromium. Only the three list modules
 * are stand-ins (their hooks read the daemon): each renders a real
 * `PickerPanel` with a different number of rows, so the popover's height
 * really changes between the tabs.
 *
 * ⚠ Tailwind does not run here, so heights that come from utilities (the
 * search field's) differ from the app's. Nothing below depends on them: every
 * assertion compares the control with itself, with the chip, or with the
 * popover's own edges.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const PICKERS_CSS = readFileSync(resolve(here, 'pickers.css'), 'utf8');

const TOKEN_NAMES = [
  '--control-compact',
  '--control-sm',
  '--control-md',
  '--radius-full',
  '--radius-element',
  '--radius-inner',
  '--radius-container',
  '--icon-chip',
  '--text-supporting',
  '--text-supporting--line-height',
  '--text-secondary',
  '--text-secondary--line-height',
  '--text-label',
  '--text-label--line-height',
  '--dur-fast-min',
  '--dur-fast',
  '--dur-fast-max',
  '--ease-out',
  '--ease-spring',
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

/** Every `@ws WS-PRIMITIVES` block of main.css (the segmented control, the rows, the switch). */
function primitiveBlocks(): string {
  const out: string[] = [];
  const re =
    /\/\* @ws WS-PRIMITIVES begin: ([^*]+?) \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: \1 \*\//g;
  for (const match of MAIN_CSS.matchAll(re)) out.push(match[2]);
  if (out.length === 0) throw new Error('main.css has no @ws WS-PRIMITIVES block');
  return out.join('\n');
}

/** Tailwind's preflight, the parts the layout depends on, plus colours that only paint. */
const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
button, input { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
svg { display: block; width: 14px; height: 14px; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
[role='listbox'] { padding: 4px; }
:root { --control-track-off: #ebebe8; --background-accent: #b85a32; --border-control: #86847f;
  --background-default: #fff; --text-on-accent: #fff; --text-muted: #666; --text-default: #111;
  --text-subtle: #777; --background-muted: #f2f2f0; --border-focus: #555; --background-focus: #ddd;
  --shadow-raised: 0 1px 2px rgb(0 0 0 / 0.1); --inset-hairline: inset 0 0 0 1px #ddd;
  --overlay-hover: rgb(0 0 0 / 0.05); --overlay-pressed: rgb(0 0 0 / 0.1); --border-subtle: #ccc; }
[data-slot='popover-content'] { background: #fff; box-shadow: 0 0 0 1px #ccc; }
`;

/**
 * The three list modules, as stand-ins: the hooks return a count, and each
 * panel is a real `PickerPanel` with `window.__rows[segment]` rows.
 */
const LIST_STUB = (segment: string, hook: string, panel: string) => `
import * as React from 'react';
import { PickerPanel, PickerRow } from './PickerPanel';

export function ${hook}() {
  // answered: the real hooks report it (V2-R7-3), and the chip shows no number
  // and no tooltip until all three have. The stand-in chip is the app's chip.
  return { activeCount: 1, visibleCount: 1, answered: true, setQuery: () => {} };
}

export function ${panel}({ inputRef }: { inputRef?: React.Ref<HTMLInputElement> }) {
  const [query, setQuery] = React.useState('');
  const count = (window as unknown as { __rows: Record<string, number> }).__rows['${segment}'];
  return (
    <PickerPanel
      listLabel="${segment}"
      searchPlaceholder="Search ${segment}…"
      query={query}
      onQueryChange={setQuery}
      scopeLabel="This chat"
      scopeHelp="Changes apply to this chat only."
      bulk={{ label: 'Disable all (' + count + ')', onClick: () => {} }}
      emptyText="Nothing here"
      inputRef={inputRef}
      data-testid="panel-${segment}"
    >
      {Array.from({ length: count }, (_, i) => (
        <PickerRow key={i} name={'${segment} ' + (i + 1)} checked={i % 2 === 0} onToggle={() => {}} />
      ))}
    </PickerPanel>
  );
}
`;

const STUBS: Record<string, string> = {
  './BottomMenuExtensionSelection': LIST_STUB(
    'extensions',
    'useExtensionPicker',
    'ExtensionsPanel'
  ),
  './BottomMenuSkillSelection': LIST_STUB('skills', 'useSkillPicker', 'SkillsPanel'),
  './BottomMenuKnowledgeSelection': LIST_STUB('knowledge', 'useKnowledgePicker', 'KnowledgePanel'),
};

const stubLists: Plugin = {
  name: 'stub-tool-lists',
  setup(builder) {
    builder.onResolve(
      { filter: /^\.\/BottomMenu(Extension|Skill|Knowledge)Selection$/ },
      (args) => ({
        path: args.path,
        namespace: 'tool-list-stub',
      })
    );
    builder.onLoad({ filter: /.*/, namespace: 'tool-list-stub' }, (args) => ({
      contents: STUBS[args.path],
      loader: 'tsx',
      resolveDir: here,
    }));
  },
};

/** A composer stand-in: the chip near the window's bottom edge, as in a chat. */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ToolsChip } from './ToolsChip';

// A test may set its own lengths before this runs.
(window as unknown as { __rows?: Record<string, number> }).__rows ??= {
  extensions: 1,
  skills: 30,
  knowledge: 4,
};

function App() {
  return (
    <div style={{ position: 'fixed', left: 40, bottom: 64 }}>
      <ToolsChip sessionId="chat-1" />
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

type Box = { top: number; bottom: number; left: number; right: number; height: number };

const EPSILON = 0.5;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

const SEGMENTS = ['extensions', 'skills', 'knowledge'] as const;
/** A tall window, and one short enough that the 30-row list must give way. */
const HEIGHTS = [900, 480] as const;

describe('the Tools popover keeps its segmented control above the chip (R-25)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the R-25 tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [stubLists],
      loader: { '.css': 'empty' },
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css = PREFLIGHT + readTokens() + primitiveBlocks() + PICKERS_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** The page with the chip, not yet open. `rows` replaces the lists' lengths. */
  async function load(height: number, rows?: Record<string, number>): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height } });
    // The open keyframe moves the surface 4px; measure the layout, not the motion.
    await tab.emulateMedia({ reducedMotion: 'reduce' });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    if (rows) {
      await tab.evaluate((r) => {
        (window as unknown as { __rows: Record<string, number> }).__rows = r;
      }, rows);
    }
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-testid="composer-tools-chip"]');
    return tab;
  }

  async function open(height: number): Promise<Page> {
    const tab = await load(height);
    await tab.click('[data-testid="composer-tools-chip"]');
    await tab.waitForSelector('[data-testid="composer-tools-tabs"]');
    await settle(tab);
    return tab;
  }

  const settle = async (tab: Page) => {
    await tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );
    await tab.waitForTimeout(250);
  };

  /** The control, the chip, the popover and the list, in viewport pixels. */
  const measure = (tab: Page, segment: string) =>
    tab.evaluate((which) => {
      const box = (el: Element | null): Box => {
        const r = el!.getBoundingClientRect();
        return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, height: r.height };
      };
      const list = document.querySelector(`[data-testid="panel-${which}"] [role="listbox"]`)!;
      return {
        control: box(
          document.querySelector('[data-testid="composer-tools-tabs"] [role="radiogroup"]')
        ),
        chip: box(document.querySelector('[data-testid="composer-tools-chip"]')),
        popover: box(document.querySelector('[data-testid="composer-tools-popover"]')),
        list: box(list),
        scrolls: list.scrollHeight > list.clientHeight + 1,
        rows: list.querySelectorAll('[role="option"]').length,
        side: document
          .querySelector('[data-testid="composer-tools-popover"]')!
          .getAttribute('data-side'),
        lastChild:
          document
            .querySelector('[data-testid="composer-tools-popover"]')!
            .lastElementChild?.getAttribute('data-testid') ?? null,
      };
    }, segment);

  /**
   * Clicks a segment at its centre and records where the control is in every
   * frame the browser paints after the click, so a jump that lasts one frame
   * fails too.
   *
   * ⚠ Which frames are painted matters. The first animation frame after the
   * click reads a layout the browser has not finished: React has committed the
   * new list, but Radix moves the popover from a ResizeObserver callback, and
   * those run after animation frames and before paint. Reading it would report
   * a jump nobody sees. Every later frame reads what the frame before it
   * painted.
   */
  async function clickAndWatch(tab: Page, segment: string) {
    const target = tab.locator(`[data-testid="tools-segment-${segment}"]`);
    const r = (await target.boundingBox())!;
    const point = { x: r.x + r.width / 2, y: r.y + r.height / 2 };
    await tab.evaluate(() => {
      const frames: number[] = [];
      (window as unknown as { __frames: number[] }).__frames = frames;
      const control = document.querySelector(
        '[data-testid="composer-tools-tabs"] [role="radiogroup"]'
      )!;
      let sinceClick = -1;
      let ticks = 0;
      window.addEventListener('click', () => (sinceClick = 0), { capture: true, once: true });
      const tick = () => {
        if (sinceClick >= 0) {
          if (sinceClick > 0) frames.push(control.getBoundingClientRect().top);
          sinceClick++;
        }
        if (frames.length < 10 && ++ticks < 600) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    await tab.mouse.click(point.x, point.y);
    await settle(tab);
    await tab.waitForFunction(
      () => (window as unknown as { __frames: number[] }).__frames.length >= 10
    );
    const frames = await tab.evaluate(() => (window as unknown as { __frames: number[] }).__frames);
    const under = await tab.evaluate(
      ({ x, y }) =>
        document.elementFromPoint(x, y)?.closest('[role="radio"]')?.getAttribute('data-testid') ??
        null,
      point
    );
    return { frames, under };
  }

  it('pins the control 14px above the chip in every list and at every window height', async (ctx) => {
    if (!browser) return ctx.skip();
    const seen: Array<{ height: number; segment: string; gap: number; left: number }> = [];
    for (const height of HEIGHTS) {
      const tab = await open(height);
      try {
        const first = await measure(tab, 'extensions');
        // The stand-in chip is the app's own: the three lists have answered, so it
        // states their sum (1 + 1 + 1) and the popover is measured against that width.
        expect(await tab.textContent('[data-testid="composer-tools-chip"]')).toBe('Tools3');
        for (const segment of SEGMENTS) {
          const { frames, under } =
            segment === 'extensions'
              ? { frames: [], under: null }
              : await clickAndWatch(tab, segment);
          const m = await measure(tab, segment);
          const where = `${segment} at ${height}px`;

          // Never flipped below the chip, and the control is the popover's last child.
          expect(m.side, where).toBe('top');
          expect(m.lastChild, where).toBe('composer-tools-tabs');
          // The popover's bottom edge is 6px above the chip; the control sits on it
          // (8px of padding), under the list.
          near(m.chip.top - m.popover.bottom, 6, `${where}: popover to chip`);
          near(m.popover.bottom - m.control.bottom, 8, `${where}: control to popover bottom`);
          expect(m.control.top, `${where}: control under the list`).toBeGreaterThanOrEqual(
            m.list.bottom - EPSILON
          );
          // ...and not one pixel away from where it was on the first list.
          near(m.control.top, first.control.top, `${where}: control top`);
          near(m.control.left, first.control.left, `${where}: control left`);
          near(m.control.right, first.control.right, `${where}: control right`);
          for (const [i, top] of frames.entries()) {
            near(top, first.control.top, `${where}: control top in painted frame ${i + 1}`);
          }
          // The pointer that clicked the segment is still on it.
          if (segment !== 'extensions') expect(under, where).toBe(`tools-segment-${segment}`);
          // The popover stays inside the window, below the band it clears.
          expect(m.popover.top, `${where}: popover top`).toBeGreaterThanOrEqual(52 - EPSILON);

          seen.push({ height, segment, gap: m.chip.top - m.control.bottom, left: m.control.left });
        }
      } finally {
        await tab.close();
      }
    }
    // The same place relative to the chip at both window heights.
    for (const s of seen) near(s.gap, 14, `${s.segment} at ${s.height}px: control to chip`);
    for (const s of seen) near(s.left, seen[0].left, `${s.segment} at ${s.height}px: left`);
  });

  it('grows and shrinks upward with the list, and scrolls a list the room cannot hold', async (ctx) => {
    if (!browser) return ctx.skip();
    const heights: Record<string, Record<string, number>> = {};
    for (const height of HEIGHTS) {
      const tab = await open(height);
      try {
        heights[height] = {};
        for (const segment of SEGMENTS) {
          if (segment !== 'extensions') await clickAndWatch(tab, segment);
          const m = await measure(tab, segment);
          heights[height][segment] = m.popover.height;
          if (segment === 'skills') {
            // 30 rows never fit: the list scrolls rather than pushing the control.
            expect(m.rows).toBe(30);
            expect(m.scrolls, `skills at ${height}px scrolls`).toBe(true);
          }
        }
      } finally {
        await tab.close();
      }
    }
    // The lists really differ in height, so the steady control above is not luck:
    // one row, thirty rows (capped) and four rows give three different popovers.
    const tall = heights[900];
    expect(tall.extensions).toBeLessThan(tall.knowledge);
    expect(tall.knowledge).toBeLessThan(tall.skills);
    // In the short window the cap is the room above the chip, not the 480px ceiling.
    expect(heights[480].skills).toBeLessThan(tall.skills);
  });

  it('opens above the chip from its first painted frame, even short of room with a long list', async (ctx) => {
    if (!browser) return ctx.skip();
    // 400px tall: the room above the chip (about 250px) is less than the 70vh the
    // cap falls back to before Radix has measured, and the list on screen at open
    // is the long one, so the first placement is the one most likely to land
    // somewhere else. Measured: it does not, with collisions on or off; this pins
    // the result rather than one mechanism.
    const tab = await load(400, { extensions: 30, skills: 1, knowledge: 1 });
    try {
      await tab.evaluate(() => {
        const frames: Array<{ side: string | null; gap: number | null }> = [];
        (window as unknown as { __open: typeof frames }).__open = frames;
        let ticks = 0;
        const tick = () => {
          const popover = document.querySelector('[data-testid="composer-tools-popover"]');
          const control = document.querySelector(
            '[data-testid="composer-tools-tabs"] [role="radiogroup"]'
          );
          const chip = document.querySelector('[data-testid="composer-tools-chip"]')!;
          // A frame reads what the frame before it painted; the first one with the
          // popover in the DOM may not have been placed yet, so it waits a frame.
          if (popover && control && popover.getAttribute('data-state') === 'open') {
            if (ticks++ > 0) {
              frames.push({
                side: popover.getAttribute('data-side'),
                gap: chip.getBoundingClientRect().top - control.getBoundingClientRect().bottom,
              });
            }
          }
          if (frames.length < 15) requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      });
      await tab.click('[data-testid="composer-tools-chip"]');
      await tab.waitForFunction(
        () => (window as unknown as { __open: unknown[] }).__open.length >= 15
      );
      const frames = await tab.evaluate(
        () => (window as unknown as { __open: Array<{ side: string | null; gap: number }> }).__open
      );
      for (const [i, frame] of frames.entries()) {
        expect(frame.side, `painted frame ${i + 1}`).toBe('top');
        near(frame.gap, 14, `painted frame ${i + 1}: control to chip`);
      }
      const m = await measure(tab, 'extensions');
      expect(m.scrolls).toBe(true);
      expect(m.popover.top).toBeGreaterThanOrEqual(52 - EPSILON);
    } finally {
      await tab.close();
    }
  });
});
