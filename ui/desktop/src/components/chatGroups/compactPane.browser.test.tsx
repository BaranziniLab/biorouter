// @vitest-environment node
/**
 * Compact panes (rung 5 of the yield ladder), in a real layout engine.
 *
 * jsdom lays nothing out, applies no stylesheet and paints nothing, so the claims that make a
 * compact pane work are only checkable here: the placeholder fits every box it can be drawn in, in
 * every family and mode; and the frame keeps the chat it hides alive. The real `CompactPane` (its
 * own measuring included) and `ChatPaneFrame` (with the real `usePaneFit`) are bundled with
 * esbuild, `styles/main.css` is compiled by Tailwind with the classes the bundle uses,
 * `compact-pane.css` and `chat-tabs.css` are appended as written, and Chromium lays them out. Only
 * the run registry and the dictation session are stood in for, at `CompactPane`'s own imports.
 *
 * Each declaration that keeps the hidden chat alive is proven the house way: the stylesheet is
 * served without it, and the defect it guards against appears.
 *
 * Set `COMPACT_PANE_SHOTS=<dir>` to write a screenshot of every placeholder box.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { compactBandControls, compactPaneDensity } from '../Layout/yieldLadder';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHOTS = process.env.COMPACT_PANE_SHOTS;
const PANE_CSS = readFileSync(resolve(here, 'compact-pane.css'), 'utf8');

interface FollowHarnessWindow extends Window {
  __chunk: (index: number) => void;
  __paragraph: () => void;
  __bottom: () => void;
  __settle: () => void;
  __isFollowing: () => boolean;
  __streamDone: boolean;
  __finishStream: () => void;
}

/** CompactPane's two process-wide sources, stood in for at its own imports only. */
const standIns: Plugin = {
  name: 'compact-pane-stand-ins',
  setup(builder) {
    builder.onResolve({ filter: /hooks\/chatStreamStore$|dictation\/dictationSession$/ }, (args) =>
      args.importer.endsWith('CompactPane.tsx')
        ? { path: args.path, namespace: 'compact-pane-stand-in' }
        : undefined
    );
    builder.onLoad({ filter: /.*/, namespace: 'compact-pane-stand-in' }, ({ path }) => ({
      contents: path.includes('chatStreamStore')
        ? 'export function useRunningChats() { return window.__running || []; }'
        : `export function subscribeDictation() { return () => {}; }
           export function dictationActivityFor() { return 'idle'; }`,
      loader: 'js',
    }));
  },
};

/** Vite's `?url&no-inline` worklet import means nothing to esbuild; the worklet is not loaded here. */
const workletUrl: Plugin = {
  name: 'worklet-url',
  setup(builder) {
    builder.onResolve({ filter: /\.worklet\.js\?/ }, () => ({
      path: 'worklet',
      namespace: 'stub',
    }));
    builder.onLoad({ filter: /.*/, namespace: 'stub' }, () => ({
      contents: 'export default "";',
      loader: 'js',
    }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { CompactPane } from './CompactPane';
import { ChatPaneFrame } from './ChatPaneFrame';
import { ChatGroupSplitter } from './ChatGroupSplitter';
import { ChatKindIcon } from '../chats/ChatKindIcon';
import MarkdownContent from '../MarkdownContent';
import { ScrollArea } from '../ui/scroll-area';
import { useTranscriptFollow, useTranscriptSettle } from '../summary/useTranscriptFollow';

const w = window as unknown as Record<string, any>;
const TITLE = 'Variant calls across the whole cohort';
// g2 sits in a column inside a row, modelled at 280 by 180 in the 1400 by 900 tree, so a 'both'
// shortfall has a plan on each axis and the action reads "Make larger", the widest of its words.
const LAYOUT = {
  kind: 'branch',
  dir: 'row',
  children: [
    { kind: 'leaf', groupId: 'g1' },
    {
      kind: 'branch',
      dir: 'col',
      children: [{ kind: 'leaf', groupId: 'g2' }, { kind: 'leaf', groupId: 'g3' }],
      sizes: [0.2, 0.8],
    },
  ],
  sizes: [0.8, 0.2],
};
const TABS = [
  { tabId: 't1', sessionId: 's1', title: TITLE, userSetName: false },
  { tabId: 't2', sessionId: 's2', title: 'Second chat', userSetName: false },
];

function Placeholder({ treeRef, insetStart = 0, box }) {
  return (
    <CompactPane
      identity={{ title: TITLE, kindSource: { name: TITLE }, tier: 'private', sessionId: 's1' }}
      tabMenu={{
        tabs: TABS,
        activeTabId: 't1',
        runningSessionIds: ['s1'],
        onSelectTab() {},
        renderGlyph: (tab) => (
          <ChatKindIcon session={{ name: tab.title }} tier="public" className="h-4 w-4" />
        ),
        titleOf: (tab) => tab.title,
      }}
      layout={LAYOUT}
      treeRef={treeRef}
      bandInsetStart={insetStart}
      onGrow={() => { w.__grew = (w.__grew || 0) + 1; }}
      onMerge={() => { w.__merged = (w.__merged || 0) + 1; }}
      groupId="g2"
      tabKey="t1"
      isActiveGroup={false}
      shortfall="both"
      animateIn={false}
      initialBox={box}
    />
  );
}

/** Every box the placeholder is held in, one cell each, in its own pane-shaped box. */
function Grid() {
  const treeRef = React.useRef(null);
  return (
    <div>
      <div ref={treeRef} style={{ width: 1400, height: 900, position: 'absolute', left: -2000 }} />
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 16, padding: 16, alignItems: 'flex-start' }}>
        {w.__cells.map(([width, height]) => (
          <div
            key={width + 'x' + height}
            className="cell"
            data-w={width}
            data-h={height}
            style={{
              width, height, display: 'flex', flexDirection: 'column',
              position: 'relative', overflow: 'hidden',
            }}
          >
            <Placeholder treeRef={treeRef} insetStart={w.__inset || 0} />
          </div>
        ))}
      </div>
    </div>
  );
}

/** Stand-in chat content: what the frame must keep alive while it is hidden. */
function Chat() {
  const [mountId] = React.useState(() => Math.random().toString(36).slice(2));
  React.useEffect(() => { w.__mounts = (w.__mounts || 0) + 1; }, []);
  return (
    <div id="chat" data-mount-id={mountId}
      style={{ display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 0, minWidth: 0 }}>
      <div id="scroller" style={{ flex: '1 1 0', minHeight: 0, overflow: 'auto' }}>
        <div style={{ height: 2000, position: 'relative' }}>
          <span id="far" style={{ position: 'absolute', left: 350, top: 10, width: 6, height: 6 }} />
        </div>
      </div>
      <textarea id="draft" defaultValue="A draft in progress" style={{ height: 40 }} />
      {/* The terminal dock's root has transition-all, so the wrapper's inherited visibility
          change animates on it: on the return it still computes hidden for a frame. */}
      <div id="dock" style={{ transition: 'all 175ms', height: 30 }}>
        <textarea id="dock-input" aria-label="Terminal input" style={{ height: 20 }} />
      </div>
      {/* A control that sets visibility: visible, as a descendant of the hidden box may: opacity
          hides it, but only inert keeps it from taking focus. */}
      <button id="visible-control" type="button" style={{ visibility: 'visible' }}>
        Visible control
      </button>
      {/* BaseChat's root is relative z-[60]: without the wrapper's opacity it would paint over
          the placeholder. This probe stands in for it, and for any rule that sets
          visibility: visible. */}
      <div id="probe"
        style={{ visibility: 'visible', position: 'relative', zIndex: 60,
          background: 'rgb(255, 0, 255)', height: 40 }} />
    </div>
  );
}

function Frame() {
  const treeRef = React.useRef(null);
  return (
    <div id="host" style={{ display: 'flex', height: 600, width: 1200 }}>
      <div id="tree" ref={treeRef} style={{ display: 'flex', flex: '1 1 0', minWidth: 0 }}>
        <div style={{ width: 300, flex: 'none' }}>left pane</div>
        <div id="slot" style={{ width: 600, height: 600, display: 'flex', minWidth: 0, minHeight: 0, flex: 'none' }}>
          <ChatPaneFrame
            data-chat-group-id="g2"
            alone={false}
            content={<Chat />}
            renderCompact={({ answer, animateIn, box }) => (
              <Placeholder treeRef={treeRef} box={box} insetStart={w.__inset || 0} />
            )}
          />
        </div>
      </div>
    </div>
  );
}

function Reserve() {
  const treeRef = React.useRef(null);
  return (
    <div>
      <div ref={treeRef} style={{ width: 1400, height: 900, position: 'absolute', left: -2000 }} />
      <div id="reserve" style={{ width: w.__reserveWidth, height: 600, display: 'flex', flexDirection: 'column' }}>
        <Placeholder treeRef={treeRef} insetStart={172} />
      </div>
    </div>
  );
}

const LINE =
  'is a line of a long reply, written long enough that it wraps in a narrow pane and rewraps ' +
  'when the pane changes width.';

/** The reply holds its own text, so a streamed chunk re-renders it alone, not the list. */
function Reply() {
  const [lines, setLines] = React.useState(() => ['The reply ' + LINE]);
  React.useEffect(() => {
    // One chunk: a new line every fourth chunk, else more words on the last one.
    w.__chunk = (n) =>
      setLines((all) =>
        n % 4 === 0
          ? [...all, all.length + ' ' + LINE]
          : [...all.slice(0, -1), all[all.length - 1] + ' and more of it ' + n]
      );
  }, []);
  return lines.map((line, i) => <p key={i} style={{ margin: '0 0 12px' }}>{line}</p>);
}

function MarkdownReply() {
  const [content, setContent] = React.useState('Synthetic compact-pane stream.\\n\\n');
  const next = React.useRef(1);
  React.useEffect(() => {
    w.__paragraph = () => {
      const addition = next.current++ + '. This synthetic reply grows while its pane changes size. ' +
        'The newest line stays in view, and an older reader keeps their place.\\n\\n';
      setContent((text) => text + addition);
    };
  }, []);
  return <MarkdownContent content={content} />;
}

const EARLIER = Array.from({ length: 30 }, (_, i) => (
  <p key={i} style={{ margin: '0 0 12px' }}>{i + ' ' + LINE}</p>
));
const COLUMN = (
  <div className="biorouter-chat-column mx-auto w-full max-w-measure-chat">
    {EARLIER}
    {w.__markdown ? <MarkdownReply /> : <Reply />}
  </div>
);

/**
 * The transcript as BaseChat mounts it: the real ScrollArea with the real follow hook, and a
 * reply that streams into a row that re-renders alone.
 */
function Transcript() {
  const scrollRef = React.useRef(null);
  const follow = useTranscriptFollow({
    scrollRef,
    isPreviewStacked: () => false,
    transcriptMounted: true,
  });
  const onRenderingComplete = useTranscriptSettle(scrollRef, 2, follow.isHolding);
  w.__settle = onRenderingComplete;
  React.useEffect(() => {
    w.__bottom = () => scrollRef.current?.scrollToBottom('auto');
    w.__isFollowing = () => scrollRef.current?.isFollowing;
  }, []);
  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 0, minWidth: 0 }}>
      <ScrollArea
        ref={scrollRef}
        className="flex-1 min-h-0 relative"
        autoScroll
        anchorBottomOnResize={follow.anchorBottomOnResize}
        onScrollChange={follow.onScrollChange}
        followGrowth={follow.followGrowth}
        paddingX={6}
        paddingY={0}
      >
        {COLUMN}
      </ScrollArea>
      <textarea id="draft" defaultValue="A draft in progress" style={{ height: 60, flex: 'none' }} />
    </div>
  );
}

function Follow() {
  const treeRef = React.useRef(null);
  const [sizes, setSizes] = React.useState([496 / 1000, 504 / 1000]);
  return (
    <div id="host" style={{ display: 'flex', height: 600, width: w.__splitter ? 1000 : 1200 }}>
      <div id="tree" ref={treeRef} style={{ display: 'flex', flex: '1 1 0', minWidth: 0 }}>
        {!w.__splitter && <div style={{ width: 300, flex: 'none' }}>left pane</div>}
        <div id="slot" style={{ width: w.__splitter ? sizes[0] * 1000 : 600, height: 600, display: 'flex', minWidth: 0, minHeight: 0, flex: 'none' }}>
          <ChatPaneFrame
            data-chat-group-id="g2"
            alone={false}
            content={<Transcript />}
            renderCompact={({ box }) => <Placeholder treeRef={treeRef} box={box} />}
          />
        </div>
        {w.__splitter && <>
          <ChatGroupSplitter dir="row" index={0} sizes={sizes} containerRef={treeRef} onResize={setSizes} />
          <div style={{ flex: '1 1 0', minWidth: 0 }}>other pane</div>
        </>}
      </div>
    </div>
  );
}

const App = { grid: Grid, frame: Frame, reserve: Reserve, follow: Follow }[w.__mode];
createRoot(document.getElementById('root')).render(<App />);
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

const WIDTHS = [40, 63, 64, 120, 140, 175, 176, 200, 290, 359];
const HEIGHTS = [72, 107, 108, 187, 188, 300, 600];
const CELLS = WIDTHS.flatMap((width) => HEIGHTS.map((height) => [width, height]));
const FAMILIES = ['parchment', 'alma-mater', 'roche-limit'] as const;

/** The frame's own rules with one declaration taken out, for the mutation cases. */
function without(declaration: RegExp, replacement = ''): string {
  const rule = ".br-chat-pane[data-pane-fit='compact'] > .br-chat-pane__content {";
  const start = PANE_CSS.indexOf(rule);
  const end = PANE_CSS.indexOf('}', start);
  const body = PANE_CSS.slice(start, end);
  const mutated = body.replace(declaration, replacement);
  if (mutated === body) throw new Error(`mutation ${declaration} matched nothing`);
  return PANE_CSS.slice(0, start) + mutated + PANE_CSS.slice(end);
}

describe('compact panes in a real layout engine (rung 5)', () => {
  let browser: Browser | null = null;
  let script = '';
  let baseCss = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      // Every geometry claim of this feature is checked only here; the jsdom suites stub the
      // boxes and would pass with the CSS wrong. So on CI a missing browser fails the file rather
      // than skipping it. The CI image has Google Chrome (ChatInput.learning.browser.test.tsx
      // already throws without one), and a developer machine without any still gets the skip.
      if (process.env.CI) {
        throw new Error(
          'No Chromium or Chrome could launch, and CI is set: the compact pane geometry tests ' +
            'must run here. Install one with `npx playwright install chromium`.'
        );
      }
      console.warn(
        'No Chromium found: SKIPPING all compact pane geometry tests. Nothing has checked the ' +
          'real layout. Install one with `npx playwright install chromium`.'
      );
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [standIns, workletUrl],
      define: { 'process.env.NODE_ENV': '"production"', 'import.meta.env': '{}' },
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
    baseCss =
      compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])) +
      '\n' +
      readFileSync(resolve(here, 'chat-tabs.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(opts: {
    mode: 'grid' | 'frame' | 'reserve' | 'follow';
    family?: (typeof FAMILIES)[number];
    dark?: boolean;
    paneCss?: string;
    globals?: Record<string, unknown>;
    viewport?: { width: number; height: number };
  }): Promise<Page> {
    const tab = await browser!.newPage({ viewport: opts.viewport ?? { width: 1300, height: 900 } });
    const globals = {
      __mode: opts.mode,
      __cells: CELLS,
      __running: [{ sessionId: 's1', chatState: 'streaming', startedAt: 1 }],
      ...opts.globals,
    };
    const theme = opts.family && opts.family !== 'parchment' ? ` data-theme="${opts.family}"` : '';
    await tab.setContent(
      `<!doctype html><html class="${opts.dark ? 'dark' : ''}"${theme}><head><style>${baseCss}\n${
        opts.paneCss ?? PANE_CSS
      }</style></head><body style="margin:0;background:var(--background-canvas)"><div id="root"></div>` +
        `<script>Object.assign(window, ${JSON.stringify(globals)});</script></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('.br-compact-pane, [data-pane-fit]');
    await tab.waitForTimeout(150);
    return tab;
  }

  /** Waits two frames, so an observer that fired on the last change has committed. */
  const settle = (tab: Page) =>
    tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );

  /** How many pixels of a screenshot of `selector` are the probe's magenta. */
  const magentaPixels = async (tab: Page, selector: string): Promise<number> => {
    const shot = await tab.locator(selector).screenshot();
    return tab.evaluate(async (b64) => {
      const img = new Image();
      img.src = 'data:image/png;base64,' + b64;
      await img.decode();
      const canvas = document.createElement('canvas');
      canvas.width = img.width;
      canvas.height = img.height;
      const g = canvas.getContext('2d')!;
      g.drawImage(img, 0, 0);
      const data = g.getImageData(0, 0, img.width, img.height).data;
      let count = 0;
      for (let i = 0; i < data.length; i += 4) {
        if (data[i] > 230 && data[i + 1] < 40 && data[i + 2] > 230) count++;
      }
      return count;
    }, shot.toString('base64'));
  };

  for (const family of FAMILIES) {
    for (const dark of [false, true]) {
      const mode = dark ? 'dark' : 'light';
      it(`fits every box it can be drawn in: ${family} ${mode}`, async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await open({
          mode: 'grid',
          family,
          dark,
          viewport: { width: 1300, height: 900 },
        });
        try {
          await settle(tab);
          const cells = await tab.evaluate(() => {
            const probe = document.createElement('div');
            probe.style.borderTop = '1px solid var(--border-danger)';
            document.body.append(probe);
            const danger = getComputedStyle(probe).borderTopColor;
            probe.remove();
            const within = (inner: DOMRect, outer: DOMRect) =>
              inner.left >= outer.left - 0.5 &&
              inner.right <= outer.right + 0.5 &&
              inner.top >= outer.top - 0.5 &&
              inner.bottom <= outer.bottom + 0.5;
            const oneLine = (el: Element | null) => {
              if (!el) return null;
              const rects = el.getClientRects();
              const lineHeight = parseFloat(getComputedStyle(el).lineHeight);
              return {
                rects: rects.length,
                height: rects[0]?.height ?? 0,
                lineHeight,
                display: getComputedStyle(el).display,
              };
            };
            return [...document.querySelectorAll<HTMLElement>('.cell')].map((cell) => {
              const pane = cell.querySelector<HTMLElement>('.br-compact-pane')!;
              const body = pane.querySelector<HTMLElement>('.br-compact-pane__body')!;
              const band = pane.querySelector<HTMLElement>('.br-compact-pane__band')!;
              const cellBox = cell.getBoundingClientRect();
              const bandBox = band.getBoundingClientRect();
              const actions = [...pane.querySelectorAll<HTMLElement>('[data-compact-pane-action]')];
              const menu = pane.querySelector<HTMLElement>(
                '[data-testid="chat-tab-overflow-trigger"]'
              );
              const tile = pane.querySelector<HTMLElement>('.br-compact-pane__tile');
              const verb = pane.querySelector<HTMLElement>('.br-compact-pane__verb');
              const colours = new Set<string>();
              for (const el of [pane, ...pane.querySelectorAll<HTMLElement>('*')]) {
                const cs = getComputedStyle(el);
                for (const value of [
                  cs.color,
                  cs.backgroundColor,
                  cs.borderTopColor,
                  cs.borderRightColor,
                  cs.borderBottomColor,
                  cs.borderLeftColor,
                  cs.fill,
                  cs.stroke,
                ]) {
                  colours.add(value);
                }
              }
              return {
                width: Number(cell.dataset.w),
                height: Number(cell.dataset.h),
                density: pane.getAttribute('data-density'),
                body: {
                  scrollWidth: body.scrollWidth,
                  clientWidth: body.clientWidth,
                  scrollHeight: body.scrollHeight,
                  clientHeight: body.clientHeight,
                },
                title: oneLine(pane.querySelector('.br-compact-pane__title')),
                state: oneLine(pane.querySelector('.br-compact-pane__state')),
                actions: actions.length,
                actionInside: actions.every((a) => within(a.getBoundingClientRect(), cellBox)),
                menu: menu ? within(menu.getBoundingClientRect(), bandBox) : null,
                bandHeight: bandBox.height,
                danger: colours.has(danger),
                tileGround: tile ? getComputedStyle(tile).backgroundColor : null,
                ground: getComputedStyle(pane).backgroundColor,
                verb: verb?.textContent ?? null,
                verbFits: verb ? verb.scrollWidth <= verb.clientWidth + 0.5 : null,
              };
            });
          });
          expect(cells).toHaveLength(CELLS.length);
          for (const cell of cells) {
            const where = `${family} ${mode} ${cell.width}x${cell.height}`;
            const density = compactPaneDensity(cell.width, cell.height);
            expect(cell.density, where).toBe(density);
            expect(cell.bandHeight, where).toBe(44);
            // Exactly one action, wholly inside the pane.
            expect(cell.actions, where).toBe(1);
            expect(cell.actionInside, where).toBe(true);
            // The ▾ is in the band whenever the band says it fits, and inside it.
            const band = compactBandControls({
              width: cell.width,
              insetStart: 0,
              density,
              tabCount: 2,
            });
            expect(cell.menu !== null, where).toBe(band.menu);
            if (cell.menu !== null) expect(cell.menu, `${where} the ▾ inside the band`).toBe(true);
            expect(cell.danger, `${where} draws no danger tone`).toBe(false);
            if (density === 'band') continue;
            expect(cell.body.scrollWidth, `${where} no sideways overflow`).toBeLessThanOrEqual(
              cell.body.clientWidth
            );
            expect(cell.body.scrollHeight, `${where} no downward overflow`).toBeLessThanOrEqual(
              cell.body.clientHeight
            );
            expect(cell.tileGround, `${where} the tile is a fill on the ground`).not.toBe(
              cell.ground
            );
            if (density === 'card') {
              expect(cell.verb, where).toBe('Make larger');
              expect(cell.verbFits, `${where} the action words fit`).toBe(true);
              for (const [name, line] of [
                ['title', cell.title],
                ['state', cell.state],
              ] as const) {
                expect(line!.rects, `${where} ${name} is one line`).toBe(1);
                expect(Math.abs(line!.height - line!.lineHeight), `${where} ${name}`).toBeLessThan(
                  1
                );
              }
            } else {
              expect(cell.title!.display, `${where} glyph hides the title`).toBe('none');
            }
          }
          if (SHOTS) {
            mkdirSync(SHOTS, { recursive: true });
            const handles = await tab.locator('.cell').all();
            for (const handle of handles) {
              const w = await handle.getAttribute('data-w');
              const h = await handle.getAttribute('data-h');
              await handle.screenshot({
                path: resolve(SHOTS, `compact-${family}-${mode}-${w}x${h}.png`),
              });
            }
          }
        } finally {
          await tab.close();
        }
      }, 60_000);
    }
  }

  const fitOf = (tab: Page) =>
    tab.evaluate(() => document.querySelector('[data-pane-fit]')!.getAttribute('data-pane-fit'));
  const resizeSlot = async (tab: Page, width: number, height: number) => {
    await tab.evaluate(
      ([w, h]) => {
        const slot = document.getElementById('slot')!;
        slot.style.width = `${w}px`;
        slot.style.height = `${h}px`;
      },
      [width, height] as const
    );
    await settle(tab);
  };

  it('crosses at 360 by 270 and returns at 372 by 282, keeping the chat exactly as it was', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ mode: 'frame' });
    try {
      await tab.evaluate(() => {
        const draft = document.getElementById('draft') as HTMLTextAreaElement;
        const scroller = document.getElementById('scroller')!;
        scroller.scrollTop = 700;
        draft.focus();
        draft.setSelectionRange(2, 7);
        // The nodes themselves, compared by identity at every step: a remount would replace them.
        (window as unknown as Record<string, unknown>).__nodes = {
          chat: document.getElementById('chat'),
          draft,
          scroller,
        };
      });
      const mountId = await tab.evaluate(() =>
        document.getElementById('chat')!.getAttribute('data-mount-id')
      );

      /** At every step, shown or hidden, the chat is the same mount with its scroll and draft. */
      const expectKept = async (where: string) => {
        const kept = await tab.evaluate(() => {
          const w = window as unknown as {
            __nodes: { chat: HTMLElement; draft: HTMLTextAreaElement; scroller: HTMLElement };
            __mounts: number;
          };
          const nodes = w.__nodes;
          const draft = nodes.draft;
          return {
            sameNodes:
              nodes.chat.isConnected &&
              nodes.chat === document.getElementById('chat') &&
              draft === document.getElementById('draft') &&
              nodes.scroller === document.getElementById('scroller'),
            mountId: nodes.chat.getAttribute('data-mount-id'),
            mounts: w.__mounts,
            scrollTop: nodes.scroller.scrollTop,
            value: draft.value,
            selection: [draft.selectionStart, draft.selectionEnd],
          };
        });
        expect(kept.sameNodes, `${where}: the same nodes`).toBe(true);
        expect(kept.mountId, `${where}: the same mount`).toBe(mountId);
        expect(kept.mounts, `${where}: mounted once`).toBe(1);
        expect(kept.scrollTop, `${where}: the transcript's scroll offset`).toBe(700);
        expect(kept.value, `${where}: the draft`).toBe('A draft in progress');
        expect(kept.selection, `${where}: the selection`).toEqual([2, 7]);
      };
      const heldBox = () =>
        tab.evaluate(() => {
          const box = document.querySelector('.br-chat-pane__content')!.getBoundingClientRect();
          return { width: box.width, height: box.height };
        });

      // Each axis one pixel either side of both seams: in under 360 (270), held from there to
      // 371 (281), out at 372 (282). A step's fit depends on the one before it, so the order is
      // the test.
      const widths: Array<[number, 'full' | 'compact']> = [
        [600, 'full'],
        [360, 'full'],
        [359, 'compact'],
        [300, 'compact'],
        [360, 'compact'],
        [371, 'compact'],
        [372, 'full'],
      ];
      for (const [width, fit] of widths) {
        await resizeSlot(tab, width, 600);
        const where = `${width} wide`;
        expect(await fitOf(tab), where).toBe(fit);
        await expectKept(where);
        if (fit === 'compact') {
          const held = await heldBox();
          expect(held.width, `${where}: the hidden chat is laid out at the 360px floor`).toBe(
            Math.max(width, 360)
          );
          expect(held.height, `${where}: and as tall as the pane`).toBe(600);
        }
        if (width === 300) {
          const compact = await tab.evaluate(() => {
            const pane = document.querySelector<HTMLElement>('[data-pane-fit]')!;
            const box = pane.getBoundingClientRect();
            const hit = document.elementFromPoint(
              box.left + box.width / 2,
              box.top + box.height / 2
            );
            const slot = document.getElementById('slot')!;
            return {
              hitInPlaceholder: !!hit?.closest('.br-compact-pane'),
              parent: { scrollWidth: slot.scrollWidth, clientWidth: slot.clientWidth },
              inert: document.querySelector('.br-chat-pane__content')!.hasAttribute('inert'),
            };
          });
          expect(compact.hitInPlaceholder).toBe(true);
          expect(compact.inert).toBe(true);
          expect(compact.parent.scrollWidth).toBeLessThanOrEqual(compact.parent.clientWidth);
          expect(await magentaPixels(tab, '#slot'), 'nothing of the hidden chat paints').toBe(0);
        }
      }
      const heights: Array<[number, 'full' | 'compact']> = [
        [600, 'full'],
        [270, 'full'],
        [269, 'compact'],
        [200, 'compact'],
        [270, 'compact'],
        [281, 'compact'],
        [282, 'full'],
      ];
      for (const [height, fit] of heights) {
        await resizeSlot(tab, 600, height);
        const where = `${height} tall`;
        expect(await fitOf(tab), where).toBe(fit);
        await expectKept(where);
        if (fit === 'compact') {
          const held = await heldBox();
          expect(held.height, `${where}: the hidden chat is laid out at the 270px floor`).toBe(
            Math.max(height, 270)
          );
          expect(held.width, `${where}: and as wide as the pane, over the width floor`).toBe(600);
        }
      }
      expect(
        await tab.evaluate(() => document.activeElement === document.getElementById('draft')),
        'focus came back to the draft'
      ).toBe(true);
    } finally {
      await tab.close();
    }
  }, 60_000);

  it('keeps following a streaming tail while compact and after restoring', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ mode: 'follow' });
    try {
      const view = () =>
        tab.evaluate(() => {
          const viewport = document.querySelector<HTMLElement>(
            '[data-radix-scroll-area-viewport]'
          )!;
          return {
            top: viewport.scrollTop,
            height: viewport.scrollHeight,
            fromEnd: viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight,
            fit: document.querySelector('[data-pane-fit]')!.getAttribute('data-pane-fit'),
          };
        });
      let nextChunk = 0;
      const stream = async (count: number) => {
        for (let i = 0; i < count; i++) {
          await tab.evaluate(
            (n) => (window as unknown as FollowHarnessWindow).__chunk(n),
            nextChunk++
          );
          await tab.waitForTimeout(30);
        }
        await tab.waitForTimeout(200);
      };
      await tab.evaluate(() => (window as unknown as FollowHarnessWindow).__bottom());
      await settle(tab);
      expect((await view()).fromEnd, 'the reader starts at the tail').toBeLessThanOrEqual(2);
      await stream(20);
      expect((await view()).fromEnd, 'the visible reply stays followed').toBeLessThanOrEqual(2);
      await tab.evaluate(() => (document.getElementById('draft') as HTMLTextAreaElement).focus());
      await resizeSlot(tab, 355, 600);
      const compact = await view();
      expect(compact.fit).toBe('compact');
      expect(compact.fromEnd, 'the width reflow keeps the reader at the tail').toBeLessThanOrEqual(
        2
      );
      for (let burst = 0; burst < 4; burst++) {
        await stream(12);
        const streamed = await view();
        expect(streamed.height).toBeGreaterThan(compact.height);
        expect(streamed.top).toBeGreaterThan(compact.top);
        expect(streamed.fromEnd, 'hidden streamed rows stay followed').toBeLessThanOrEqual(2);
      }
      await resizeSlot(tab, 400, 600);
      expect((await view()).fit).toBe('full');
      expect((await view()).fromEnd, 'the latest line is in view on restore').toBeLessThanOrEqual(
        2
      );
      expect(await tab.locator('#draft').inputValue()).toBe('A draft in progress');
      expect(await tab.locator('#draft').evaluate((node) => node === document.activeElement)).toBe(
        true
      );
    } finally {
      await tab.close();
    }
  }, 60_000);

  it('keeps a reader above the tail in place while compact content streams', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ mode: 'follow' });
    try {
      await tab.evaluate(() => {
        const viewport = document.querySelector<HTMLElement>('[data-radix-scroll-area-viewport]')!;
        viewport.scrollTop = 500;
      });
      await tab.waitForTimeout(150);
      await resizeSlot(tab, 355, 600);
      const top = await tab
        .locator('[data-radix-scroll-area-viewport]')
        .evaluate((node) => node.scrollTop);
      for (let n = 1; n <= 8; n++) {
        await tab.evaluate((i) => (window as unknown as FollowHarnessWindow).__chunk(i), n);
        await tab.waitForTimeout(150);
      }
      const after = await tab.locator('[data-radix-scroll-area-viewport]').evaluate((node) => ({
        top: node.scrollTop,
        fromEnd: node.scrollHeight - node.scrollTop - node.clientHeight,
      }));
      expect(
        Math.abs(after.top - top),
        'a hidden reader is not pulled away from older text'
      ).toBeLessThanOrEqual(2);
      expect(after.fromEnd).toBeGreaterThan(200);
    } finally {
      await tab.close();
    }
  }, 60_000);

  it('resumes following a live stream after the reader returns to the tail, including compact restoration', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ mode: 'follow', globals: { __markdown: true } });
    try {
      await resizeSlot(tab, 496, 600);
      await tab.evaluate(() => (window as unknown as FollowHarnessWindow).__settle());
      await tab.evaluate(() => {
        const viewport = document.querySelector<HTMLElement>('[data-radix-scroll-area-viewport]')!;
        viewport.scrollTop -= 400;
      });
      await tab.waitForTimeout(150);
      await tab.evaluate(() => {
        const viewport = document.querySelector<HTMLElement>('[data-radix-scroll-area-viewport]')!;
        viewport.scrollTop = viewport.scrollHeight;
      });
      await tab.waitForTimeout(150);
      const stream = (count: number) =>
        tab.evaluate(async (count) => {
          const harness = window as unknown as FollowHarnessWindow;
          for (let i = 0; i < count; i++) {
            harness.__paragraph();
            await new Promise((done) => setTimeout(done, 40));
          }
        }, count);
      const expectTail = async (where: string) => {
        await tab.waitForTimeout(200);
        const fromEnd = await tab
          .locator('[data-radix-scroll-area-viewport]')
          .evaluate(
            (viewport) => viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight
          );
        expect(fromEnd, where).toBeLessThanOrEqual(2);
      };
      await stream(20);
      await expectTail('the visible stream follows after the reader returns to its tail');
      await tab.evaluate(() => {
        const draft = document.getElementById('draft') as HTMLTextAreaElement;
        draft.focus();
        draft.setSelectionRange(2, 9);
      });
      await resizeSlot(tab, 355, 600);
      expect(await fitOf(tab)).toBe('compact');
      await stream(40);
      await expectTail('the compact stream follows after the reader returns to its tail');
      await resizeSlot(tab, 367, 600);
      expect(await fitOf(tab)).toBe('compact');
      await stream(20);
      await resizeSlot(tab, 374, 600);
      expect(await fitOf(tab)).toBe('full');
      await stream(20);
      await expectTail('the restored stream follows its final line');
      expect(await tab.locator('#draft').evaluate((node) => node === document.activeElement)).toBe(
        true
      );
      expect(
        await tab
          .locator('#draft')
          .evaluate((node: HTMLTextAreaElement) => [node.selectionStart, node.selectionEnd])
      ).toEqual([2, 9]);
    } finally {
      await tab.close();
    }
  }, 60_000);

  for (const finishAtResize of [false, true]) {
    it(`keeps a running Markdown tail followed through an actual splitter drag${finishAtResize ? ' when its final chunk follows the resize' : ''}`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open({ mode: 'follow', globals: { __markdown: true, __splitter: true } });
      try {
        await tab.evaluate(() => {
          const harness = window as unknown as FollowHarnessWindow;
          harness.__settle();
          const draft = document.getElementById('draft') as HTMLTextAreaElement;
          draft.focus();
          draft.setSelectionRange(2, 9);
          harness.__streamDone = false;
          let chunks = 0;
          const timer = setInterval(() => {
            harness.__paragraph();
            if (++chunks === 120) {
              clearInterval(timer);
              harness.__streamDone = true;
            }
          }, 40);
          harness.__finishStream = () => {
            clearInterval(timer);
            harness.__paragraph();
            harness.__streamDone = true;
          };
        });
        await tab.waitForTimeout(800);
        await tab.evaluate(() => {
          const viewport = document.querySelector<HTMLElement>(
            '[data-radix-scroll-area-viewport]'
          )!;
          viewport.scrollTop = viewport.scrollHeight;
        });
        await tab.waitForTimeout(200);
        const drag = async (delta: number) => {
          const box = await tab
            .getByRole('separator', { name: 'Resize chat groups' })
            .boundingBox();
          if (!box) throw new Error('No splitter box.');
          const x = box.x + box.width / 2;
          const y = box.y + box.height / 2;
          await tab.mouse.move(x, y);
          await tab.mouse.down();
          await tab.mouse.move(x + delta, y);
          await tab.mouse.up();
          await settle(tab);
        };
        await drag(-141);
        expect(await fitOf(tab)).toBe('compact');
        if (finishAtResize) {
          await tab.evaluate(() => (window as unknown as FollowHarnessWindow).__finishStream());
          await tab.waitForTimeout(200);
          const gap = await tab
            .locator('[data-radix-scroll-area-viewport]')
            .evaluate(
              (viewport) => viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight
            );
          expect(
            gap,
            'a final chunk just after reflow stays followed while compact'
          ).toBeLessThanOrEqual(2);
        }
        await tab.waitForTimeout(850);
        expect(
          await tab.evaluate(() => (window as unknown as FollowHarnessWindow).__isFollowing()),
          'a width reflow during the real splitter drag is not the reader scrolling away'
        ).toBe(true);
        await drag(12);
        expect(await fitOf(tab)).toBe('compact');
        await drag(7);
        expect(await fitOf(tab)).toBe('full');
        await tab.waitForFunction(() => (window as unknown as FollowHarnessWindow).__streamDone);
        await tab.waitForTimeout(200);
        const fromEnd = await tab
          .locator('[data-radix-scroll-area-viewport]')
          .evaluate(
            (viewport) => viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight
          );
        expect(fromEnd, 'the final streamed line is visible after restoration').toBeLessThanOrEqual(
          2
        );
        expect(
          await tab.locator('#draft').evaluate((node) => node === document.activeElement)
        ).toBe(true);
      } finally {
        await tab.close();
      }
    }, 60_000);
  }

  it('keeps focus on the placeholder itself when its band has no room for an action, and gives it back', async (ctx) => {
    if (!browser) return ctx.skip();
    // 150 by 100 beside the 172px titlebar reserve: band density, and no room in the band for the
    // action or the ▾, so the placeholder has nothing focusable inside it.
    const tab = await open({ mode: 'frame', globals: { __inset: 172 } });
    try {
      await tab.evaluate(() => {
        const draft = document.getElementById('draft') as HTMLTextAreaElement;
        draft.focus();
        draft.setSelectionRange(3, 3);
      });
      await resizeSlot(tab, 150, 100);
      expect(await fitOf(tab)).toBe('compact');
      const compact = await tab.evaluate(() => {
        const placeholder = document.querySelector<HTMLElement>('.br-compact-pane')!;
        return {
          density: placeholder.getAttribute('data-density'),
          focusables: placeholder.querySelectorAll('button, [tabindex="0"]').length,
          onPlaceholder: document.activeElement === placeholder,
          onBody: document.activeElement === document.body,
          edge: getComputedStyle(placeholder, '::after').boxShadow,
        };
      });
      expect(compact.density).toBe('band');
      expect(compact.focusables, 'nothing in the placeholder can take focus').toBe(0);
      expect(compact.onBody, 'focus did not fall to the page').toBe(false);
      expect(compact.onPlaceholder, 'the placeholder holds it').toBe(true);
      // The shared focus-region edge, repeated above the band (a keyboard person sees where it went).
      expect(compact.edge).not.toBe('none');
      await resizeSlot(tab, 600, 600);
      expect(await fitOf(tab)).toBe('full');
      const back = await tab.evaluate(() => {
        const draft = document.getElementById('draft') as HTMLTextAreaElement;
        return {
          focused: document.activeElement === draft,
          selection: [draft.selectionStart, draft.selectionEnd],
        };
      });
      expect(back.focused, 'focus came back to the draft').toBe(true);
      expect(back.selection).toEqual([3, 3]);
    } finally {
      await tab.close();
    }
  }, 60_000);

  it('gives focus back to a control in a box that transitions visibility, as the terminal dock does', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ mode: 'frame' });
    try {
      // The precondition, on its own: when an inherited visibility change lands on a box with
      // transition: all, the control inside still computes hidden in that frame and refuses
      // focus. Without it this case would prove nothing.
      const bare = await tab.evaluate(() => {
        const outer = document.createElement('div');
        outer.style.visibility = 'hidden';
        outer.innerHTML = '<div style="transition: all 175ms"><textarea></textarea></div>';
        document.body.appendChild(outer);
        const input = outer.querySelector('textarea')!;
        void getComputedStyle(input).visibility;
        outer.style.visibility = 'visible';
        input.focus();
        const result = {
          visibility: getComputedStyle(input).visibility,
          refused: document.activeElement !== input,
        };
        outer.remove();
        return result;
      });
      expect(bare.visibility).toBe('hidden');
      expect(bare.refused).toBe(true);

      await tab.evaluate(() => document.getElementById('dock-input')!.focus());
      await resizeSlot(tab, 300, 600);
      expect(await fitOf(tab)).toBe('compact');
      expect(
        await tab.evaluate(() => !!document.activeElement?.hasAttribute('data-compact-pane-action'))
      ).toBe(true);
      // Longer than the dock's 175ms: a return inside it would reverse a transition still at
      // visible, and the case would pass with nothing settled.
      await tab.waitForTimeout(400);
      await resizeSlot(tab, 600, 600);
      expect(await fitOf(tab)).toBe('full');
      const back = await tab.evaluate(() => ({
        focused: document.activeElement === document.getElementById('dock-input'),
        onBody: document.activeElement === document.body,
      }));
      expect(back.onBody, 'focus did not fall to the page').toBe(false);
      expect(back.focused, 'focus came back to the control in the dock').toBe(true);
    } finally {
      await tab.close();
    }
  }, 60_000);

  it('absorbs a scrollIntoView inside the hidden chat, and scrolls nothing outside the pane', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ mode: 'frame' });
    try {
      await resizeSlot(tab, 300, 600);
      expect(await fitOf(tab)).toBe('compact');
      await tab.evaluate(() => document.getElementById('far')!.scrollIntoView());
      await settle(tab);
      const scrolls = await tab.evaluate(() => {
        const pane = document.querySelector<HTMLElement>('[data-pane-fit]')!;
        const ancestors: number[] = [];
        for (let el = pane.parentElement; el; el = el.parentElement) ancestors.push(el.scrollLeft);
        return {
          pane: pane.scrollLeft,
          ancestors,
          page: document.scrollingElement!.scrollLeft,
          window: window.scrollX,
        };
      });
      expect(scrolls.ancestors.every((left) => left === 0)).toBe(true);
      expect(scrolls.page).toBe(0);
      expect(scrolls.window).toBe(0);
      expect(scrolls.pane, 'the pane reset its own scroll').toBe(0);
    } finally {
      await tab.close();
    }
  }, 60_000);

  it('commits the placeholder before the frame that first sees the small box paints', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ mode: 'frame' });
    try {
      const seen = await tab.evaluate(
        () =>
          new Promise<boolean>((done) => {
            const pane = document.querySelector<HTMLElement>('[data-pane-fit]')!;
            let resized = false;
            // Observers are called in the order they were created, so this one runs after the
            // pane's own, in the same rendering step and before that frame's paint.
            const watcher = new ResizeObserver((entries) => {
              const width = entries[0].contentRect.width;
              if (width > 400) {
                // The first observation, at 600: only now shrink the slot, in the next frame.
                if (!resized) {
                  resized = true;
                  requestAnimationFrame(() => {
                    document.getElementById('slot')!.style.width = '300px';
                  });
                }
                return;
              }
              watcher.disconnect();
              done(!!pane.querySelector('.br-compact-pane'));
            });
            watcher.observe(pane);
          })
      );
      expect(seen).toBe(true);
    } finally {
      await tab.close();
    }
  }, 60_000);

  describe('each declaration that keeps the hidden chat alive is load-bearing', () => {
    it('without opacity: 0, the hidden chat paints over the placeholder', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open({ mode: 'frame', paneCss: without(/\s*opacity:\s*0;/) });
      try {
        await resizeSlot(tab, 300, 600);
        expect(await fitOf(tab)).toBe('compact');
        expect(await magentaPixels(tab, '#slot')).toBeGreaterThan(0);
      } finally {
        await tab.close();
      }
    }, 60_000);

    it('without the max() floors, the hidden chat shrinks to the sliver the pane is', async (ctx) => {
      if (!browser) return ctx.skip();
      const cases = [
        {
          axis: 'width',
          css: without(/width:\s*max\(100%,\s*var\(--pane-min-width\)\);/, 'width: 100%;'),
          pane: [300, 600],
          expected: 300,
        },
        {
          axis: 'height',
          css: without(/height:\s*max\(100%,\s*var\(--pane-min-height\)\);/, 'height: 100%;'),
          pane: [600, 200],
          expected: 200,
        },
      ] as const;
      for (const { axis, css, pane, expected } of cases) {
        const tab = await open({ mode: 'frame', paneCss: css });
        try {
          await resizeSlot(tab, pane[0], pane[1]);
          expect(await fitOf(tab), axis).toBe('compact');
          const box = await tab.evaluate(() => {
            const rect = document.querySelector('.br-chat-pane__content')!.getBoundingClientRect();
            return { width: rect.width, height: rect.height };
          });
          expect(box[axis], `${axis}: the floor is gone`).toBe(expected);
        } finally {
          await tab.close();
        }
      }
    }, 60_000);

    /**
     * `visibility: hidden` already keeps an ordinary control (the draft) from taking focus, so a
     * focus() on the draft proves nothing about `inert`. What only `inert` stops is a descendant
     * that sets `visibility: visible`: `opacity: 0` hides it from the eye, not from focus. React
     * sets `inert` from ChatPaneFrame, so the mutation takes the attribute off in the page.
     */
    it('without inert, a control that sets visibility: visible takes focus inside the hidden chat', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open({ mode: 'frame' });
      try {
        await resizeSlot(tab, 300, 600);
        expect(await fitOf(tab)).toBe('compact');
        const tryFocus = () =>
          tab.evaluate(() => {
            const draft = document.getElementById('draft')!;
            const visible = document.getElementById('visible-control')!;
            draft.focus();
            const draftTook = document.activeElement === draft;
            visible.focus();
            const visibleTook = document.activeElement === visible;
            (document.activeElement as HTMLElement | null)?.blur();
            return { draftTook, visibleTook };
          });
        const real = await tryFocus();
        expect(real.draftTook, 'the draft cannot take focus while hidden').toBe(false);
        expect(real.visibleTook, 'nor can a control that sets visibility: visible').toBe(false);
        await tab.evaluate(() => {
          document.querySelector('.br-chat-pane__content')!.removeAttribute('inert');
        });
        const mutated = await tryFocus();
        expect(mutated.draftTook, 'visibility: hidden alone still holds the draft').toBe(false);
        expect(mutated.visibleTook, 'without inert the visible control takes focus').toBe(true);
      } finally {
        await tab.close();
      }
    }, 60_000);

    it('without overflow: hidden on the pane, the held box spills out of it', async (ctx) => {
      if (!browser) return ctx.skip();
      const css = PANE_CSS.replace(
        /(\.br-chat-pane\[data-pane-fit='compact'\] \{\s*)overflow:\s*hidden;/,
        '$1'
      );
      expect(css).not.toBe(PANE_CSS);
      const tab = await open({ mode: 'frame', paneCss: css });
      try {
        await resizeSlot(tab, 300, 600);
        const parent = await tab.evaluate(() => {
          const slot = document.getElementById('slot')!;
          return { scrollWidth: slot.scrollWidth, clientWidth: slot.clientWidth };
        });
        expect(parent.scrollWidth).toBeGreaterThan(parent.clientWidth);
      } finally {
        await tab.close();
      }
    }, 60_000);

    /**
     * `display: none` was the obvious way to hide the chat, and it would hand every observer in it
     * a 0px box (the transcript's follow, the composer's growth, the terminal's fit) and a 0
     * scroll offset for as long as the pane is compact. Chrome 157 restores the offset when the
     * box comes back (measured), so the defect is asserted while hidden, where it bites.
     */
    it('with display: none for visibility: hidden, the hidden chat has no box and no scroll', async (ctx) => {
      if (!browser) return ctx.skip();
      const read = (tab: Page) =>
        tab.evaluate(() => {
          const scroller = document.getElementById('scroller')!;
          return {
            height: scroller.clientHeight,
            width: document.querySelector('.br-chat-pane__content')!.getBoundingClientRect().width,
            scrollTop: scroller.scrollTop,
          };
        });
      const mutated = await open({
        mode: 'frame',
        paneCss: without(/visibility:\s*hidden;/, 'display: none;'),
      });
      const real = await open({ mode: 'frame' });
      try {
        for (const tab of [mutated, real]) {
          await tab.evaluate(() => {
            document.getElementById('scroller')!.scrollTop = 700;
          });
          await resizeSlot(tab, 300, 600);
          expect(await fitOf(tab)).toBe('compact');
        }
        const broken = await read(mutated);
        expect(broken.width).toBe(0);
        expect(broken.height).toBe(0);
        expect(broken.scrollTop).toBe(0);
        const kept = await read(real);
        expect(kept.width).toBe(360);
        expect(kept.height).toBeGreaterThan(0);
        expect(kept.scrollTop).toBe(700);
      } finally {
        await mutated.close();
        await real.close();
      }
    }, 60_000);
  });

  it('leaves the titlebar reserve free: the ▾ sits right of 172px, and not at all under 208', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const [width, menu] of [
      [208, true],
      [207, false],
    ] as const) {
      const tab = await open({ mode: 'reserve', globals: { __reserveWidth: width } });
      try {
        await settle(tab);
        const band = await tab.evaluate(() => {
          const pane = document.getElementById('reserve')!.getBoundingClientRect();
          const trigger = document.querySelector('[data-testid="chat-tab-overflow-trigger"]');
          const controls = document.querySelectorAll('.br-compact-pane__band button');
          return {
            left: trigger ? trigger.getBoundingClientRect().left - pane.left : null,
            controls: controls.length,
          };
        });
        if (menu) {
          expect(band.left, `${width}px`).not.toBeNull();
          expect(band.left!, `${width}px`).toBeGreaterThanOrEqual(172);
        } else {
          expect(band.controls, `${width}px`).toBe(0);
        }
      } finally {
        await tab.close();
      }
    }
  }, 60_000);
});
