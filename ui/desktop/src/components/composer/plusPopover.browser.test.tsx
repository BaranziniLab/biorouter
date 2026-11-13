// @vitest-environment node
/**
 * R-25 (owner message 13), on the `+` popover (owner, 2026-11-13: the `+` replaced the old `+`
 * menu AND the Tools chip): the popover's segmented control (Add | Extensions | Skills |
 * Knowledge) sits on the popover's bottom edge, directly above the `+`, and stays at the same
 * place whatever the list on screen and whatever the window's height, so switching from Add to
 * Extensions to Skills to Knowledge (and back) never moves the control under the pointer that
 * clicked it.
 *
 * Only a layout engine can say that: jsdom places nothing, and the claim is about Radix's
 * positioning (a `side="top"` popover is pinned by its bottom edge, collisions off) meeting
 * `pickers.css` (the cap from the room above) and `composer.css` (the popover's width). So the
 * REAL `ComposerPlusMenu`, `toolsLists`, `Popover`, `SegmentedControl` and `PickerPanel` are
 * bundled with esbuild and rendered in Chromium. Only the three list modules are stand-ins
 * (their hooks read the daemon): each renders a real `PickerPanel` with a different number of
 * rows, so the popover's height really changes between the lists, and the extensions stand-in
 * answers the app's "show this chat's extensions" request the way the real hook does.
 *
 * ⚠ Tailwind does not run here, so sizes that come from utilities (the search field's) differ
 * from the app's; the `+`'s own 32px box is shimmed below so the trigger is the app's size.
 * Nothing below depends on the rest: every assertion compares the control with itself, with the
 * `+`, or with the popover's own edges.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const BOTTOM_MENU = resolve(here, '../bottom_menu');
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const PICKERS_CSS = readFileSync(resolve(BOTTOM_MENU, 'pickers.css'), 'utf8');
const COMPOSER_CSS = readFileSync(resolve(here, 'composer.css'), 'utf8');

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

/**
 * Tailwind's preflight, the parts the layout depends on, plus colours that only paint, and the
 * `+` button's own box (`size="default" shape="round"`: `w-control-md h-control-md`, centred).
 */
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
[data-testid='composer-plus'] { display: inline-flex; align-items: center; justify-content: center;
  width: var(--control-md); height: var(--control-md); }
`;

/**
 * The three list modules, as stand-ins: the hooks return a count, and each panel is a real
 * `PickerPanel` with `window.__rows[segment]` rows. The extensions hook also answers the app's
 * "show this chat's extensions" event, as the real one does.
 */
const LIST_STUB = (segment: string, hook: string, panel: string) => `
import * as React from 'react';
import { PickerPanel, PickerRow } from './PickerPanel';
import { OPEN_CURRENT_CHAT_EXTENSIONS_EVENT } from '../../utils/sessionToolEvents';

export function ${hook}(
  sessionId: string | null,
  { onOpenRequest }: { onOpenRequest?: () => void } = {}
) {
  const request = React.useRef(onOpenRequest);
  request.current = onOpenRequest;
  React.useEffect(() => {
    if ('${segment}' !== 'extensions') return;
    const open = (event: Event) => {
      if ((event as CustomEvent<{ sessionId: string }>).detail?.sessionId === sessionId) {
        request.current?.();
      }
    };
    window.addEventListener(OPEN_CURRENT_CHAT_EXTENSIONS_EVENT, open);
    return () => window.removeEventListener(OPEN_CURRENT_CHAT_EXTENSIONS_EVENT, open);
  }, [sessionId]);
  // answered: the real hooks report it (V2-R7-3), and the \`+\` states no counts until all
  // three have.
  return { activeCount: 1, visibleCount: 1, answered: true, setQuery: () => {} };
}

export function ${panel}({ inputRef }: { inputRef?: React.Ref<HTMLInputElement> }) {
  const [query, setQuery] = React.useState('');
  const count = (window as unknown as { __rows: Record<string, number> }).__rows['${segment}'];
  // A test may give a row a note or a label (the knowledge list's state-saying name).
  const extras = (
    window as unknown as { __extras?: Record<string, Array<{ note?: string; label?: string }>> }
  ).__extras?.['${segment}'];
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
        <PickerRow
          key={i}
          name={'${segment} ' + (i + 1)}
          checked={i % 2 === 0}
          onToggle={() => {}}
          {...extras?.[i]}
        />
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
      resolveDir: BOTTOM_MENU,
    }));
  },
};

/** A composer stand-in: the `+` near the window's bottom edge, as in a chat. */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ComposerPlusMenu } from './ComposerPlusMenu';
import { openCurrentChatExtensions } from '../../utils/sessionToolEvents';

// A test may set its own lengths before this runs.
(window as unknown as { __rows?: Record<string, number> }).__rows ??= {
  extensions: 1,
  skills: 30,
  knowledge: 4,
};
// The app's "show this chat's extensions" request (the extensions list's own door).
(window as unknown as { __openExtensions: () => void }).__openExtensions = () =>
  openCurrentChatExtensions('chat-1');

function App() {
  return (
    <div style={{ position: 'fixed', left: 40, bottom: 64 }}>
      <ComposerPlusMenu
        sessionId="chat-1"
        onAttachFiles={() => {}}
        onInsertTrigger={() => {}}
        onOpenDecisionMonitors={() => {}}
      />
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

const EPSILON = 0.5;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

type Segment = 'add' | 'extensions' | 'skills' | 'knowledge';
/** Every list, in the order a person would walk them, and back to where a click opens. */
const WALK: Segment[] = ['add', 'extensions', 'skills', 'knowledge', 'add'];
/** A tall window, and one short enough that the 30-row list must give way. */
const HEIGHTS = [900, 480] as const;

const segmentTestId = (segment: Segment) =>
  segment === 'add' ? 'plus-segment-add' : `tools-segment-${segment}`;

describe('the + popover keeps its segmented control above the + (R-25)', () => {
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
    // In the order `ComposerPlusMenu` imports them: the `+` popover's rules re-cut the picker's.
    css = PREFLIGHT + readTokens() + primitiveBlocks() + PICKERS_CSS + COMPOSER_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** The page with the `+`, not yet open. `rows` replaces the lists' lengths. */
  async function load(
    height: number,
    rows?: Record<string, number>,
    extras?: Record<string, Array<{ note?: string; label?: string }>>
  ): Promise<Page> {
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
    if (extras) {
      await tab.evaluate((e) => {
        (window as unknown as { __extras: typeof e }).__extras = e;
      }, extras);
    }
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-testid="composer-plus"]');
    return tab;
  }

  async function open(height: number): Promise<Page> {
    const tab = await load(height);
    await tab.click('[data-testid="composer-plus"]');
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

  /** The control, the `+`, the popover and the list on screen, in viewport pixels. */
  const measure = (tab: Page, segment: Segment) =>
    tab.evaluate((which) => {
      const box = (el: Element | null) => {
        const r = el!.getBoundingClientRect();
        return {
          top: r.top,
          bottom: r.bottom,
          left: r.left,
          right: r.right,
          width: r.width,
          height: r.height,
        };
      };
      // Add is a menu of rows; the three tool lists are list boxes.
      const list = document.querySelector(
        which === 'add'
          ? '[data-testid="composer-plus-add-panel"] [role="menu"]'
          : `[data-testid="panel-${which}"] [role="listbox"]`
      )!;
      const popover = document.querySelector('[data-testid="composer-plus-popover"]')!;
      return {
        control: box(
          document.querySelector('[data-testid="composer-tools-tabs"] [role="radiogroup"]')
        ),
        plus: box(document.querySelector('[data-testid="composer-plus"]')),
        popover: box(popover),
        list: box(list),
        scrolls: list.scrollHeight > list.clientHeight + 1,
        rows: list.querySelectorAll(which === 'add' ? '[role="menuitem"]' : '[role="option"]')
          .length,
        side: popover.getAttribute('data-side'),
        segment: popover.getAttribute('data-segment'),
        lastChild: popover.lastElementChild?.getAttribute('data-testid') ?? null,
      };
    }, segment);

  /**
   * Clicks a segment at its centre and records where the control is in every frame the browser
   * paints after the click, so a jump that lasts one frame fails too.
   *
   * ⚠ Which frames are painted matters. The first animation frame after the click reads a
   * layout the browser has not finished: React has committed the new list, but Radix moves the
   * popover from a ResizeObserver callback, and those run after animation frames and before
   * paint. Reading it would report a jump nobody sees. Every later frame reads what the frame
   * before it painted.
   */
  async function clickAndWatch(tab: Page, segment: Segment) {
    const target = tab.locator(`[data-testid="${segmentTestId(segment)}"]`);
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

  it('pins the control 14px above the + in every list and at every window height', async (ctx) => {
    if (!browser) return ctx.skip();
    const seen: Array<{ height: number; segment: Segment; gap: number; left: number }> = [];
    for (const height of HEIGHTS) {
      const tab = await open(height);
      try {
        const plus = tab.locator('[data-testid="composer-plus"]');
        // The counts the Tools chip carried are the `+`'s name; it shows no words of its own.
        expect(await plus.getAttribute('aria-label')).toBe(
          'Add files and tools. Tools: 1 extension, 1 skill, 1 knowledge base'
        );
        expect(await plus.textContent()).toBe('');
        // A click opens on Add.
        const first = await measure(tab, 'add');
        expect(first.segment).toBe('add');
        expect(first.rows).toBe(4);
        for (const [step, segment] of WALK.entries()) {
          const { frames, under } =
            step === 0 ? { frames: [], under: null } : await clickAndWatch(tab, segment);
          const m = await measure(tab, segment);
          const where = `${segment} (step ${step}) at ${height}px`;

          expect(m.segment, where).toBe(segment);
          // Never flipped below the `+`, and the control is the popover's last child.
          expect(m.side, where).toBe('top');
          expect(m.lastChild, where).toBe('composer-tools-tabs');
          // The popover's bottom edge is 6px above the `+`; the control sits on it (8px of
          // padding), under the list.
          near(m.plus.top - m.popover.bottom, 6, `${where}: popover to +`);
          near(m.popover.bottom - m.control.bottom, 8, `${where}: control to popover bottom`);
          expect(m.control.top, `${where}: control under the list`).toBeGreaterThanOrEqual(
            m.list.bottom - EPSILON
          );
          // Opened on the composer card's left edge, 9px left of the `+` (the card's 1px border
          // and 8px padding, `PLUS_INSET_IN_CARD` in ComposerPlusMenu.tsx), at the width that
          // keeps four whole words.
          near(m.popover.left, m.plus.left - 9, `${where}: popover left`);
          near(m.popover.width, 344, `${where}: popover width`);
          // ...and not one pixel away from where it was on the first list.
          near(m.control.top, first.control.top, `${where}: control top`);
          near(m.control.left, first.control.left, `${where}: control left`);
          near(m.control.right, first.control.right, `${where}: control right`);
          for (const [i, top] of frames.entries()) {
            near(top, first.control.top, `${where}: control top in painted frame ${i + 1}`);
          }
          // The pointer that clicked the segment is still on it.
          if (step > 0) expect(under, where).toBe(segmentTestId(segment));
          // The popover stays inside the window, below the band it clears.
          expect(m.popover.top, `${where}: popover top`).toBeGreaterThanOrEqual(52 - EPSILON);

          seen.push({ height, segment, gap: m.plus.top - m.control.bottom, left: m.control.left });
        }
      } finally {
        await tab.close();
      }
    }
    // The same place relative to the `+` at both window heights.
    for (const s of seen) near(s.gap, 14, `${s.segment} at ${s.height}px: control to +`);
    for (const s of seen) near(s.left, seen[0].left, `${s.segment} at ${s.height}px: left`);
  });

  it('grows and shrinks upward with the list, and scrolls a list the room cannot hold', async (ctx) => {
    if (!browser) return ctx.skip();
    const heights: Record<string, Partial<Record<Segment, number>>> = {};
    for (const height of HEIGHTS) {
      const tab = await open(height);
      try {
        heights[height] = {};
        for (const [step, segment] of WALK.entries()) {
          if (step > 0) await clickAndWatch(tab, segment);
          const m = await measure(tab, segment);
          heights[height][segment] = m.popover.height;
          if (segment === 'add') {
            // Attach files, Mention, Commands and Decision models and monitors: always whole.
            expect(m.rows).toBe(4);
            expect(m.scrolls, `add at ${height}px does not scroll`).toBe(false);
          }
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
    // The lists really differ in height, so the steady control above is not luck: one row,
    // thirty rows (capped) and four rows give three different popovers, and Add a fourth.
    const tall = heights[900];
    expect(tall.extensions!).toBeLessThan(tall.knowledge!);
    expect(tall.knowledge!).toBeLessThan(tall.skills!);
    expect(tall.add!).not.toBe(tall.skills!);
    // In the short window the cap is the room above the `+`, not the 480px ceiling.
    expect(heights[480].skills!).toBeLessThan(tall.skills!);
  });

  /**
   * UI defect 3 (final check, 2026-10-12): every row's switch, in all three tool lists, is named
   * by the row's name, and Chromium's own accessibility tree (what a screen reader is handed)
   * still has one node per row: the option, named once, with no switch beside it.
   */
  it('names every switch by its row, and leaves one announcement per row in Chromium’s tree', async (ctx) => {
    if (!browser) return ctx.skip();
    const NOTE = 'Unavailable in this chat (public model)';
    // The knowledge list names a row by a label that says its state.
    const label = (i: number) =>
      `knowledge ${i}, ${i % 2 === 1 ? 'visible to' : 'hidden from'} this chat`;
    const tab = await load(
      900,
      { extensions: 3, skills: 2, knowledge: 2 },
      { extensions: [{}, { note: NOTE }], knowledge: [{ label: label(1) }, { label: label(2) }] }
    );
    try {
      const cdp = await tab.context().newCDPSession(tab);
      await cdp.send('Accessibility.enable');
      await tab.click('[data-testid="composer-plus"]');
      await tab.waitForSelector('[data-testid="composer-tools-tabs"]');
      await settle(tab);
      for (const [segment, count] of [
        ['extensions', 3],
        ['skills', 2],
        ['knowledge', 2],
      ] as const) {
        await clickAndWatch(tab, segment);
        const panel = tab.getByTestId(`panel-${segment}`);
        // Out of the tree: no switch is reachable as one...
        expect(await panel.getByRole('switch').count(), `${segment}: switches`).toBe(0);
        expect(await panel.getByRole('option').count(), `${segment}: rows`).toBe(count);
        const { nodes } = await cdp.send('Accessibility.getFullAXTree');
        const exposed = nodes.filter((node) => !node.ignored);
        expect(
          exposed.filter((node) => node.role?.value === 'switch'),
          `${segment}: switches Chromium exposes`
        ).toHaveLength(0);
        for (let i = 1; i <= count; i++) {
          const name = `${segment} ${i}`;
          // ...and yet each is named by its row, in a real engine.
          const picture = panel.getByRole('switch', { name, exact: true, includeHidden: true });
          expect(await picture.count(), `the switch named ${name}`).toBe(1);
          expect(
            await picture.evaluate((el) => el.closest('[role="option"]')?.textContent),
            name
          ).toMatch(new RegExp(`^${name}`));
          // One announcement: one node carries the row's name, and it is the option, which says
          // it once (with its note, or as its label says it).
          const option =
            segment === 'knowledge'
              ? label(i)
              : segment === 'extensions' && i === 2
                ? `${name} ${NOTE}`
                : name;
          const named = exposed.filter(
            (node) =>
              typeof node.name?.value === 'string' &&
              node.name.value.includes(name) &&
              !['StaticText', 'InlineTextBox'].includes(String(node.role?.value))
          );
          expect(
            named.map((node) => [node.role?.value, node.name?.value]),
            `${segment}: nodes named ${name}`
          ).toEqual([['option', option]]);
        }
      }
    } finally {
      await tab.close();
    }
  });

  /** Records, from the first painted open frame, which side the popover is on and the gap. */
  async function watchOpen(tab: Page) {
    await tab.evaluate(() => {
      const frames: Array<{ side: string | null; gap: number | null }> = [];
      (window as unknown as { __open: typeof frames }).__open = frames;
      let ticks = 0;
      const tick = () => {
        const popover = document.querySelector('[data-testid="composer-plus-popover"]');
        const control = document.querySelector(
          '[data-testid="composer-tools-tabs"] [role="radiogroup"]'
        );
        const plus = document.querySelector('[data-testid="composer-plus"]')!;
        // A frame reads what the frame before it painted; the first one with the popover in
        // the DOM may not have been placed yet, so it waits a frame.
        if (popover && control && popover.getAttribute('data-state') === 'open') {
          if (ticks++ > 0) {
            frames.push({
              side: popover.getAttribute('data-side'),
              gap: plus.getBoundingClientRect().top - control.getBoundingClientRect().bottom,
            });
          }
        }
        if (frames.length < 15) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
  }

  const openFrames = async (tab: Page) => {
    await tab.waitForFunction(
      () => (window as unknown as { __open: unknown[] }).__open.length >= 15
    );
    return tab.evaluate(
      () => (window as unknown as { __open: Array<{ side: string | null; gap: number }> }).__open
    );
  };

  it('opens above the + from its first painted frame, on Add', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load(400, { extensions: 30, skills: 1, knowledge: 1 });
    try {
      await watchOpen(tab);
      await tab.click('[data-testid="composer-plus"]');
      for (const [i, frame] of (await openFrames(tab)).entries()) {
        expect(frame.side, `painted frame ${i + 1}`).toBe('top');
        near(frame.gap, 14, `painted frame ${i + 1}: control to +`);
      }
      const m = await measure(tab, 'add');
      expect(m.segment).toBe('add');
      expect(m.popover.top).toBeGreaterThanOrEqual(52 - EPSILON);
    } finally {
      await tab.close();
    }
  });

  it('opens above the + from its first painted frame, even short of room with a long list', async (ctx) => {
    if (!browser) return ctx.skip();
    // 400px tall: the room above the `+` (about 250px) is less than the 70vh the cap falls back
    // to before Radix has measured, and the "show this chat's extensions" request opens the
    // popover straight on the long list, so the first placement is the one most likely to land
    // somewhere else. Measured: it does not; this pins the result rather than one mechanism.
    const tab = await load(400, { extensions: 30, skills: 1, knowledge: 1 });
    try {
      await watchOpen(tab);
      await tab.evaluate(() =>
        (window as unknown as { __openExtensions: () => void }).__openExtensions()
      );
      for (const [i, frame] of (await openFrames(tab)).entries()) {
        expect(frame.side, `painted frame ${i + 1}`).toBe('top');
        near(frame.gap, 14, `painted frame ${i + 1}: control to +`);
      }
      const m = await measure(tab, 'extensions');
      expect(m.segment).toBe('extensions');
      expect(m.scrolls).toBe(true);
      expect(m.popover.top).toBeGreaterThanOrEqual(52 - EPSILON);
    } finally {
      await tab.close();
    }
  });
});
