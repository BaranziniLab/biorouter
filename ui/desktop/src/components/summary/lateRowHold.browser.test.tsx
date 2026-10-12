// @vitest-environment node
/**
 * A turn error row the person did not cause, landing after they could be
 * reading, is added without moving what is on screen (vision V2-R9-2).
 *
 * Measured in the running app on an 82-figure chat whose model cannot be
 * restored: the transcript is drawn at 6.0 s, the agent load's
 * "Model could not be prepared" row lands at the end at 12.5 s, and in the next
 * frame every card moves 62px up, under the pointer. The scroll area followed
 * the growth because the reader was at the end.
 *
 * jsdom lays nothing out, so the real `ProgressiveMessageList`, `ScrollArea`
 * (with BaseChat's `autoScroll`, follow anchor and `followGrowth`),
 * `useTranscriptFollow`, `useTranscriptSettle` and `useLateRowHold` are bundled
 * with esbuild and driven in Chromium, in a frame shaped like BaseChat's
 * column, with a 62px stand-in for the error row (24px above it, 38px tall).
 * The first case is the instrument: today's wiring, given the same steps, moves
 * the rows in view.
 *
 * `ScrollArea`'s `followGrowth` is requested from WS-PRIMITIVES
 * (requests/WS-PRIMITIVES/WS-SUMMARY-3.md). Until it lands, the bundle applies
 * exactly that diff to the file it reads (`withFollowGrowth`), and a file that
 * already has it is bundled as it is.
 */
import { readFile } from 'node:fs/promises';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));

/** The requested `ScrollArea` change, verbatim (WS-SUMMARY-3). */
const FOLLOW_GROWTH_DIFF: ReadonlyArray<readonly [string, string]> = [
  [
    '  anchorBottomOnResize?: () => boolean;\n}',
    '  anchorBottomOnResize?: () => boolean;\n  followGrowth?: () => boolean;\n}',
  ],
  [
    '      anchorBottomOnResize,\n      ...props',
    '      anchorBottomOnResize,\n      followGrowth,\n      ...props',
  ],
  [
    '    const BOTTOM_SCROLL_THRESHOLD = 200;',
    '    const followGrowthRef = React.useRef(followGrowth);\n    followGrowthRef.current = followGrowth;\n\n    const BOTTOM_SCROLL_THRESHOLD = 200;',
  ],
  [
    '        !isActivelyScrollingRef.current\n      ) {',
    '        !isActivelyScrollingRef.current &&\n        (followGrowthRef.current?.() ?? true)\n      ) {',
  ],
  [
    '          if (viewportRef.current && !isActivelyScrollingRef.current) {',
    '          if (\n            viewportRef.current &&\n            !isActivelyScrollingRef.current &&\n            (followGrowthRef.current?.() ?? true)\n          ) {',
  ],
];

const withFollowGrowth: Plugin = {
  name: 'scroll-area-follow-growth',
  setup(pluginBuild) {
    pluginBuild.onLoad({ filter: /[\\/]ui[\\/]scroll-area\.tsx$/ }, async (args) => {
      let contents = await readFile(args.path, 'utf8');
      if (!contents.includes('followGrowth')) {
        for (const [from, to] of FOLLOW_GROWTH_DIFF) {
          if (contents.split(from).length !== 2) {
            throw new Error(
              `scroll-area.tsx changed: the requested diff no longer applies at ${from}`
            );
          }
          contents = contents.replace(from, to);
        }
      }
      return { contents, loader: 'tsx' };
    });
  },
};

/** The app's Tailwind classes on these elements, as plain rules. */
const PAGE_CSS = `
  body { margin: 0; font: 14px/20px sans-serif; }
  .relative { position: relative; }
  .overflow-hidden { overflow: hidden; }
  .h-full { height: 100%; }
  .w-full { width: 100%; }
  .px-6 { padding-left: 24px; padding-right: 24px; }
  .flex { display: flex; } .justify-center { justify-content: center; }
  .pt-7 { padding-top: 28px; } .mb-6 { margin-bottom: 24px; } .mt-6 { margin-top: 24px; }
  .br-scroll-viewport > div { display: block !important; }
  #frame { display: flex; flex-direction: column; width: 1100px; height: 700px; }
  .transcript { flex: 1 1 0%; min-height: 0; }
  .chat-column { max-width: 760px; margin: 0 auto; }
  .row { margin: 0; padding-top: 8px; }
  #error-row { height: 38px; }
  #composer { flex-shrink: 0; height: 120px; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import ProgressiveMessageList from '../ProgressiveMessageList';
import { ScrollArea } from '../ui/scroll-area';
import { useTranscriptFollow, useTranscriptSettle } from './useTranscriptFollow';
import { useLateRowHold } from './useLateRowHold';

window.electron = { platform: 'darwin' };

const message = (i) => ({
  id: 'm' + i,
  role: i % 2 ? 'assistant' : 'user',
  created: 1700000000 + i,
  metadata: { userVisible: true, agentVisible: true },
  content: [{ type: 'text', text: 'message ' + i }],
});
const rowHeight = (index) => 40 + (index % 5) * 30;
const restoreFailure = () => ({
  message: 'Could not create provider: Configuration value not found: AZURE_OPENAI_ENDPOINT',
  code: 'provider_restore_failed',
  scope: 'session',
  retryable: false,
});
const noHold = { holdForLateRow: () => {}, releaseHold: () => {} };

function Frame({ legacy, count, batchDelay }) {
  const all = React.useMemo(() => Array.from({ length: count }, (_, i) => message(i)), [count]);
  const scrollRef = React.useRef(null);
  const isStacked = React.useCallback(() => false, []);
  const [messages, setMessages] = React.useState([]);
  const [turnError, setTurnError] = React.useState(null);
  const [running, setRunning] = React.useState(false);
  React.useEffect(() => {
    const timer = setTimeout(() => setMessages(all), 50);
    return () => clearTimeout(timer);
  }, [all]);
  const follow = useTranscriptFollow({ scrollRef, isPreviewStacked: isStacked, transcriptMounted: true });
  const settle = useTranscriptSettle(scrollRef, messages.length, legacy ? undefined : follow.isHolding);
  useLateRowHold({
    sessionId: 's1',
    transcriptShown: messages.length > 0,
    row: turnError,
    turnRunning: running,
    follow: legacy ? noHold : follow,
  });
  window.chat = {
    fail: () => setTurnError(restoreFailure()),
    // A send: the store clears the row and runs a turn; BaseChat asks for the end.
    send: () => {
      setTurnError(null);
      setRunning(true);
      setMessages((m) => [...m, message(m.length)]);
      setTimeout(() => scrollRef.current.scrollToBottom(), 200);
    },
    append: () => setMessages((m) => [...m, message(m.length)]),
    toBottom: () => scrollRef.current.scrollToBottom('auto'),
  };
  const renderMessage = (m, index) => (
    <p key={m.id} id={'row-' + index} className="row" style={{ height: rowHeight(index) }}>
      {index}. {m.content[0].text}
    </p>
  );
  return (
    <div id="frame">
      <ScrollArea
        ref={scrollRef}
        className="transcript"
        autoScroll
        anchorBottomOnResize={follow.anchorBottomOnResize}
        onScrollChange={follow.onScrollChange}
        followGrowth={legacy ? undefined : follow.followGrowth}
        paddingX={6}
      >
        <div className="chat-column">
          <ProgressiveMessageList
            messages={messages}
            chat={{ sessionId: 's1' }}
            isUserMessage={(m) => m.role === 'user'}
            renderMessage={renderMessage}
            chatState="idle"
            batchSize={20}
            batchDelay={batchDelay}
            showLoadingThreshold={50}
            onRenderingComplete={() => {
              window.settled = (window.settled || 0) + 1;
              settle();
            }}
            onOpenArtifact={() => {}}
            onRunInTerminal={null}
          />
          {turnError && <div id="error-row" className="mt-6">Model could not be prepared</div>}
        </div>
      </ScrollArea>
      <div id="composer" />
    </div>
  );
}

window.mount = (options) =>
  createRoot(document.getElementById('root')).render(<Frame {...options} />);
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

interface MountOptions {
  legacy?: boolean;
  count?: number;
  batchDelay?: number;
}

type ChatWindow = Window & {
  chat: { fail(): void; send(): void; append(): void; toBottom(): void };
  settled?: number;
  tops?: number[];
};

describe('a late row the person did not cause moves nothing on screen', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the late row tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [withFollowGrowth],
      define: { 'process.env.NODE_ENV': '"production"', 'import.meta.env': '{}' },
      loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open({ legacy = false, count = 60, batchDelay = 20 }: MountOptions) {
    const tab = await browser!.newPage({ viewport: { width: 1200, height: 800 } });
    tab.setDefaultTimeout(8000);
    const errors: string[] = [];
    tab.on('pageerror', (error) => errors.push(String(error)));
    await tab.setContent(
      `<!doctype html><html><head><style>${PAGE_CSS}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    expect(errors, 'the bundle runs').toEqual([]);
    await tab.evaluate(
      (mount) => (window as unknown as { mount(o: MountOptions): void }).mount(mount),
      { legacy, count, batchDelay }
    );
    await tab.waitForSelector(`#row-${count - 1}`);
    return tab;
  }

  /** Content end to viewport bottom: 0 is the newest line against the composer. */
  const gap = (tab: Page) =>
    tab.evaluate(() => {
      const viewport = document.querySelector<HTMLElement>('.br-scroll-viewport')!;
      return Math.round(viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight);
    });

  /** The last message's top edge in the viewport. */
  const lastTop = (tab: Page, count: number) =>
    tab.evaluate((i) => {
      const viewport = document.querySelector<HTMLElement>('.br-scroll-viewport')!;
      const row = document.getElementById(`row-${i}`)!;
      return Math.round(row.getBoundingClientRect().top - viewport.getBoundingClientRect().top);
    }, count - 1);

  const frames = (tab: Page, n = 2) =>
    tab.evaluate(
      (n) =>
        new Promise<void>((done) => {
          let left = n;
          const step = () => (--left <= 0 ? done() : requestAnimationFrame(step));
          requestAnimationFrame(step);
        }),
      n
    );

  /** Until the list has settled `times` times, then two frames and a beat for a scroll. */
  async function settled(tab: Page, rows: number, times = 1) {
    await tab.waitForFunction(
      ({ rows, times }) =>
        document.querySelectorAll('.row').length === rows &&
        ((window as unknown as ChatWindow).settled ?? 0) >= times,
      { rows, times }
    );
    await frames(tab);
    await tab.waitForTimeout(150);
  }

  /**
   * The last message's top in every frame from now until `ms` later: the row
   * is added in the first of them.
   */
  async function watchLastTop(tab: Page, count: number, ms: number, act: () => Promise<void>) {
    await tab.evaluate((i) => {
      const w = window as unknown as ChatWindow;
      w.tops = [];
      const viewport = document.querySelector<HTMLElement>('.br-scroll-viewport')!;
      const sample = () => {
        const row = document.getElementById(`row-${i}`);
        if (row) {
          w.tops!.push(
            Math.round(row.getBoundingClientRect().top - viewport.getBoundingClientRect().top)
          );
        }
        requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
    }, count - 1);
    await act();
    await tab.waitForSelector('#error-row');
    await tab.waitForTimeout(ms);
    return tab.evaluate(() => [...new Set((window as unknown as ChatWindow).tops!)]);
  }

  const fail = (tab: Page) => tab.evaluate(() => (window as unknown as ChatWindow).chat.fail());

  it('today’s wiring moves every row 62px up when the row lands', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ legacy: true });
    await settled(tab, 60);
    await tab.waitForTimeout(400);
    const before = await lastTop(tab, 60);
    const tops = await watchLastTop(tab, 60, 300, () => fail(tab));
    expect(tops, 'the instrument moved the rows in view').toContain(before - 62);
    expect(await gap(tab), 'and followed the row into view').toBeLessThanOrEqual(1);
    await tab.close();
  }, 20_000);

  it('adds the row below the fold, with no row on screen moving in any frame', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({});
    await settled(tab, 60);
    await tab.waitForTimeout(400);
    const before = await lastTop(tab, 60);
    const tops = await watchLastTop(tab, 60, 400, () => fail(tab));
    expect(tops).toEqual([before]);
    expect(await gap(tab), 'the row waits just below the fold').toBe(62);
    // A later re-render that grows nothing, and a message's settle, follow nothing.
    await tab.evaluate(() => window.dispatchEvent(new Event('resize')));
    await frames(tab, 3);
    expect(await lastTop(tab, 60)).toBe(before);
    await tab.close();
  }, 20_000);

  it('keeps it there while older batches are still landing above', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ batchDelay: 300 });
    // The newest batch is drawn at the end; older ones follow every 300ms.
    await tab.waitForTimeout(600);
    const before = await lastTop(tab, 60);
    const tops = await watchLastTop(tab, 60, 1200, () => fail(tab));
    await settled(tab, 60);
    expect(tops).toEqual([before]);
    expect(await lastTop(tab, 60)).toBe(before);
    expect(await gap(tab)).toBe(62);
    await tab.close();
  }, 20_000);

  it('follows again once the reader reaches the row', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({});
    await settled(tab, 60);
    await tab.waitForTimeout(400);
    await fail(tab);
    await tab.waitForSelector('#error-row');
    await frames(tab, 3);
    await tab.evaluate(() => {
      const viewport = document.querySelector<HTMLElement>('.br-scroll-viewport')!;
      viewport.scrollTop = viewport.scrollHeight;
    });
    await frames(tab, 3);
    expect(await gap(tab)).toBeLessThanOrEqual(1);
    await tab.evaluate(() => (window as unknown as ChatWindow).chat.append());
    await settled(tab, 61, 2);
    expect(await gap(tab), 'new content is followed as before').toBeLessThanOrEqual(1);
    await tab.close();
  }, 20_000);

  it('a send takes the reader to the end', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({});
    await settled(tab, 60);
    await tab.waitForTimeout(400);
    await fail(tab);
    await tab.waitForSelector('#error-row');
    await frames(tab, 3);
    await tab.evaluate(() => (window as unknown as ChatWindow).chat.send());
    await tab.waitForTimeout(700);
    expect(await gap(tab)).toBeLessThanOrEqual(1);
    await tab.close();
  }, 20_000);

  it('follows a row that lands while the chat is still opening', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({});
    // Inside the grace: the transcript was drawn a moment ago.
    await fail(tab);
    await tab.waitForSelector('#error-row');
    await settled(tab, 60);
    expect(await gap(tab), 'the row joined the open, in view').toBeLessThanOrEqual(1);
    await tab.close();
  }, 20_000);

  it('a short chat shows the row in the room under its end, moving nothing', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ count: 4 });
    await settled(tab, 4);
    await tab.waitForTimeout(400);
    const before = await lastTop(tab, 4);
    const tops = await watchLastTop(tab, 4, 300, () => fail(tab));
    expect(tops).toEqual([before]);
    const inView = await tab.evaluate(() => {
      const viewport = document.querySelector<HTMLElement>('.br-scroll-viewport')!;
      const row = document.getElementById('error-row')!.getBoundingClientRect();
      return row.bottom <= viewport.getBoundingClientRect().bottom;
    });
    expect(inView).toBe(true);
    // A row in view holds nothing: what arrives next is followed as before,
    // also once the chat grows past the viewport.
    for (let i = 0; i < 8; i++) {
      await tab.evaluate(() => (window as unknown as ChatWindow).chat.append());
    }
    await settled(tab, 12, 2);
    expect(await gap(tab), 'later content is followed').toBeLessThanOrEqual(1);
    await tab.close();
  }, 20_000);
});
