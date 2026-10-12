// @vitest-environment node
/**
 * The live chat keeps its newest line in view when the transcript changes size
 * while the reader follows it (vision round 6, V2-R6-1).
 *
 * Measured in the app: opening any subagent chat left it 32px short of its last
 * line (the "Subagent of …" line mounts above the scroller after the open has
 * scrolled to the bottom), and a window made 200px shorter hid the newest 200px.
 * The scroll area's bottom anchor was engaged only under a stacked preview.
 *
 * jsdom lays nothing out and runs no ResizeObserver, so the real `ScrollArea`
 * and `useTranscriptFollow` are bundled with esbuild and driven in Chromium, in
 * a frame shaped like BaseChat's column: an optional subagent line, the
 * transcript (the 760px column of text, `paddingX={6}`), the composer. The
 * frame's size is set straight on the DOM, so no React render happens and the
 * scroll area's content-growth auto-scroll (the accident that sometimes saved
 * the old wiring) cannot hide a missing anchor. The first case is the
 * instrument: the old wiring, given the same steps, ends 32px short.
 */
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));

/** The app's Tailwind classes on these elements, as plain rules. */
const PAGE_CSS = `
  body { margin: 0; font: 14px/20px sans-serif; }
  .relative { position: relative; }
  .overflow-hidden { overflow: hidden; }
  .h-full { height: 100%; }
  .w-full { width: 100%; }
  .px-6 { padding-left: 24px; padding-right: 24px; }
  .br-scroll-viewport > div { display: block !important; }
  #frame { display: flex; flex-direction: column; width: 1100px; height: 700px; }
  #line { flex-shrink: 0; height: 32px; }
  #sheet { flex-shrink: 0; height: 300px; }
  .transcript { flex: 1 1 0%; min-height: 0; }
  .chat-column { max-width: 760px; margin: 0 auto; }
  .chat-column p { margin: 0 0 16px; }
  #composer { flex-shrink: 0; height: 120px; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ScrollArea } from '../ui/scroll-area';
import { useTranscriptFollow } from './useTranscriptFollow';

const WORDS = 'the cohort was filtered to adults with two or more visits and a recorded diagnosis before the index date';
const ROWS = Array.from({ length: 60 }, (_, i) =>
  <p key={i} id={'row-' + i}>{i + 1}. {WORDS} {WORDS.split(' ').slice(0, i % 9).join(' ')}</p>
);

function Frame({ legacy }) {
  const scrollRef = React.useRef(null);
  const stackedRef = React.useRef(false);
  const isStacked = React.useCallback(() => stackedRef.current, []);
  const [line, setLine] = React.useState(false);
  const [sheet, setSheet] = React.useState(false);
  const follow = legacy ? null : useTranscriptFollow({
    scrollRef,
    isPreviewStacked: isStacked,
    transcriptMounted: true,
  });
  window.chat = {
    scrollToBottom: () => scrollRef.current.scrollToBottom('auto'),
    showLine: () => setLine(true),
    openStackedSheet: () => { stackedRef.current = true; setSheet(true); },
    keepPlace: () => follow.keepPlaceForWidthChange(),
  };
  // One element for the life of the page: no render reaches the auto-scroll.
  const column = React.useMemo(() => <div className="chat-column">{ROWS}</div>, []);
  return (
    <div id="frame">
      {line && <div id="line">Subagent of GWAS follow-up</div>}
      {sheet && <div id="sheet">Preview</div>}
      <ScrollArea
        ref={scrollRef}
        className="transcript"
        autoScroll
        anchorBottomOnResize={legacy ? isStacked : follow.anchorBottomOnResize}
        onScrollChange={legacy ? undefined : follow.onScrollChange}
        paddingX={6}
      >
        {column}
      </ScrollArea>
      <div id="composer" />
    </div>
  );
}

createRoot(document.getElementById('root')).render(<Frame legacy={window.LEGACY === true} />);
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

interface Place {
  /** Content end to viewport bottom: 0 is the newest line against the composer. */
  gap: number;
  top: number;
  /** The last row's bottom edge minus the viewport's bottom edge (> 0 is cut off). */
  lastRowBelow: number;
}

describe('the live transcript follows its newest line through a resize (V2-R6-1)', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the transcript follow tests.');
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
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function openChat(options: { legacy?: boolean } = {}): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1200, height: 800 } });
    tab.setDefaultTimeout(8000);
    await tab.setContent(
      `<!doctype html><html><head><style>${PAGE_CSS}</style></head><body><div id="root"></div></body></html>`
    );
    if (options.legacy) await tab.evaluate(() => ((window as { LEGACY?: boolean }).LEGACY = true));
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#row-59');
    // The open lands on the newest message, as BaseChat's first render does.
    await tab.evaluate(() =>
      (window as unknown as { chat: { scrollToBottom(): void } }).chat.scrollToBottom()
    );
    await frames(tab);
    return tab;
  }

  /** Long enough for layout, the resize observers and the scroll events they cause. */
  const frames = (tab: Page) =>
    tab.evaluate(
      () =>
        new Promise<void>((done) =>
          requestAnimationFrame(() =>
            requestAnimationFrame(() => requestAnimationFrame(() => done()))
          )
        )
    );

  const call = (tab: Page, method: string) =>
    tab.evaluate(
      (name) => (window as unknown as { chat: Record<string, () => void> }).chat[name](),
      method
    );

  /**
   * `measure` reads the layout in the same task, as the column glide and the
   * clicked-card restore do in a layout effect: the browser's scroll anchoring
   * then moves `scrollTop` before the resize observers run.
   */
  const resizeFrame = async (
    tab: Page,
    size: { width?: number; height?: number; measure?: boolean }
  ) => {
    await tab.evaluate(({ width, height, measure }) => {
      const frame = document.getElementById('frame')!;
      if (width !== undefined) frame.style.width = `${width}px`;
      if (height !== undefined) frame.style.height = `${height}px`;
      if (measure) document.getElementById('row-30')!.getBoundingClientRect();
    }, size);
    await frames(tab);
  };

  const scrollTo = async (tab: Page, top: number) => {
    await tab.evaluate((value) => {
      document.querySelector<HTMLElement>('.br-scroll-viewport')!.scrollTop = value;
    }, top);
    await frames(tab);
  };

  const place = (tab: Page): Promise<Place> =>
    tab.evaluate(() => {
      const viewport = document.querySelector<HTMLElement>('.br-scroll-viewport')!;
      const last = document.getElementById('row-59')!;
      return {
        gap: Math.round(viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight),
        top: Math.round(viewport.scrollTop),
        lastRowBelow: Math.round(
          last.getBoundingClientRect().bottom - viewport.getBoundingClientRect().bottom
        ),
      };
    });

  it('the old wiring (anchored only under a stacked preview) ends 32px short', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openChat({ legacy: true });
    expect((await place(tab)).gap).toBe(0);
    await call(tab, 'showLine');
    await frames(tab);
    expect((await place(tab)).gap, 'the instrument sees the bug').toBe(32);
    await tab.close();
  });

  it('keeps the newest line in view when the subagent line mounts after the open', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openChat();
    await call(tab, 'showLine');
    await frames(tab);
    const after = await place(tab);
    expect(after.gap).toBe(0);
    expect(after.lastRowBelow).toBeLessThanOrEqual(0);
    await tab.close();
  });

  it('keeps the end through a shorter window, and a shorter narrower one', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openChat();
    await resizeFrame(tab, { height: 500 });
    expect((await place(tab)).gap, '700 to 500 tall').toBe(0);
    await resizeFrame(tab, { width: 760, height: 420 });
    expect((await place(tab)).gap, '1100x500 to 760x420').toBe(0);
    await tab.close();
  });

  it('keeps the end when a narrower window wraps the text taller', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openChat();
    const before = await tab.evaluate(
      () => document.querySelector<HTMLElement>('.br-scroll-viewport')!.scrollHeight
    );
    await resizeFrame(tab, { width: 560 });
    const grew = await tab.evaluate(
      () => document.querySelector<HTMLElement>('.br-scroll-viewport')!.scrollHeight
    );
    expect(grew, 'the text re-wrapped taller').toBeGreaterThan(before + 100);
    expect((await place(tab)).gap).toBe(0);
    await tab.close();
  });

  it('keeps the end when the narrowing is measured before the observers run', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openChat();
    await resizeFrame(tab, { width: 560, measure: true });
    expect((await place(tab)).gap).toBe(0);
    await tab.close();
  });

  it('leaves a reader who scrolled up where they are', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openChat();
    const end = (await place(tab)).top;
    await scrollTo(tab, end - 600);
    await resizeFrame(tab, { height: 500 });
    const shorter = await place(tab);
    expect(shorter.top, 'a shorter window keeps the top line').toBe(end - 600);
    await resizeFrame(tab, { width: 560 });
    expect((await place(tab)).gap, 'a narrower one does not pull them down').toBeGreaterThan(400);
    await tab.close();
  });

  it('does not pull a reader a little above the end down to it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openChat();
    const end = (await place(tab)).top;
    await scrollTo(tab, end - 100);
    // Wider track, same 760px column: nothing re-wraps, nothing may move.
    await resizeFrame(tab, { width: 900 });
    expect((await place(tab)).top).toBe(end - 100);
    // Shorter: the line against the composer stays against it.
    await resizeFrame(tab, { height: 600 });
    expect((await place(tab)).gap).toBe(100);
    await tab.close();
  });

  it('keeps a clicked card in place for one width change, then follows again', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openChat();
    await call(tab, 'keepPlace');
    await resizeFrame(tab, { width: 560 });
    const held = await place(tab);
    expect(held.gap, 'the held change leaves the end where the reflow put it').toBeGreaterThan(0);
    // Back at the end, the next width change follows again: the hold has expired.
    await call(tab, 'scrollToBottom');
    await frames(tab);
    await resizeFrame(tab, { width: 480 });
    expect((await place(tab)).gap).toBe(0);
    await tab.close();
  });

  it('still anchors under a stacked preview when the reader has scrolled up', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openChat();
    const end = (await place(tab)).top;
    await scrollTo(tab, end - 600);
    await call(tab, 'openStackedSheet');
    await frames(tab);
    const after = await place(tab);
    expect(after.top, 'the bottom line stays put: the top moves by the sheet').toBe(end - 300);
    await tab.close();
  });
});
