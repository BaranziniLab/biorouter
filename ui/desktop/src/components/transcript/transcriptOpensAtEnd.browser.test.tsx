// @vitest-environment node
/**
 * A long live chat opens at its end and stays there while its older messages
 * load (vision V2-R7-2, checklist 12: a jump on a tab switch).
 *
 * `ProgressiveMessageList` used to draw a long chat's OLDEST 20 messages first
 * and append newer batches below, so a tab switch painted the chat's first
 * question, then its middle, then its end (52 messages), and an 82-figure chat
 * sat away from its end for 1.6 s. It now draws the newest batch first and adds
 * older batches above while the bottom stays where it is.
 *
 * jsdom lays nothing out, so the real list, `ScrollArea` (with BaseChat's
 * `autoScroll` and follow anchor) and `useTranscriptFollow` are bundled with
 * esbuild and driven in Chromium, in a frame shaped like BaseChat's column. The
 * place is sampled after every DOM change (a MutationObserver callback runs
 * after React's commit, layout effects included, and before the browser can
 * paint) and on every animation frame. The first case is the instrument: the
 * old order, given the same frame, paints a bottom that is not the last
 * message.
 */
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const COUNT = 60;
const LAST = `row-${COUNT - 1}`;

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
  #composer { flex-shrink: 0; height: 120px; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import ProgressiveMessageList from '../ProgressiveMessageList';
import { ScrollArea } from '../ui/scroll-area';
import { useTranscriptFollow } from '../summary/useTranscriptFollow';

window.electron = { platform: 'darwin' };

const MESSAGES = Array.from({ length: ${COUNT} }, (_, i) => ({
  id: 'm' + i,
  role: i % 2 ? 'assistant' : 'user',
  created: 1700000000 + i,
  metadata: { userVisible: true, agentVisible: true },
  content: [{ type: 'text', text: 'message ' + i }],
}));

// Rows of different heights, as figure cards and short questions are; or all
// one line, so that the newest batch fits in the viewport.
const rowHeight = (index, short) => (short ? 16 : 40 + (index % 5) * 30);

function Frame({ opensAt, arriveLater, batchDelay, short }) {
  const renderMessage = (message, index) => (
    <p key={message.id} id={'row-' + index} className="row" style={{ height: rowHeight(index, short) }}>
      {index}. {message.content[0].text}
    </p>
  );
  const scrollRef = React.useRef(null);
  const isStacked = React.useCallback(() => false, []);
  const [messages, setMessages] = React.useState(arriveLater ? [] : MESSAGES);
  const follow = useTranscriptFollow({ scrollRef, isPreviewStacked: isStacked, transcriptMounted: true });
  const firstRef = React.useRef(true);
  // BaseChat's handleRenderingComplete: the first settle lands on the end.
  const onRenderingComplete = React.useCallback(() => {
    window.settled = (window.settled || 0) + 1;
    if (firstRef.current) {
      firstRef.current = false;
      scrollRef.current?.scrollToBottom('auto');
    }
  }, []);
  window.chat = { load: () => setMessages(MESSAGES) };
  return (
    <div id="frame">
      <ScrollArea
        ref={scrollRef}
        className="transcript"
        autoScroll
        anchorBottomOnResize={follow.anchorBottomOnResize}
        onScrollChange={follow.onScrollChange}
        paddingX={6}
      >
        <div className="chat-column">
          <ProgressiveMessageList
            messages={messages}
            chat={{ sessionId: 's1' }}
            isUserMessage={(m) => m.role === 'user'}
            renderMessage={renderMessage}
            chatState={arriveLater ? 'loadingConversation' : 'idle'}
            opensAt={opensAt}
            batchSize={20}
            batchDelay={batchDelay}
            showLoadingThreshold={50}
            onRenderingComplete={onRenderingComplete}
            onOpenArtifact={() => {}}
            onRunInTerminal={null}
          />
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

interface Sample {
  /** Rows drawn. */
  rows: number;
  /** The chat's last message is drawn. */
  hasLast: boolean;
  /** Content end to viewport bottom: 0 is the end in view. */
  gap: number;
  /** The scroller can scroll at all (more content than viewport). */
  overflows: boolean;
  when: 'mutation' | 'frame';
}

interface MountOptions {
  opensAt?: 'start' | 'end';
  arriveLater?: boolean;
  batchDelay?: number;
  short?: boolean;
}

describe('a long live chat opens at its end (V2-R7-2)', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the transcript open tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"', 'import.meta.env': '{}' },
      loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Opens the frame with a recorder running from before the first commit. */
  async function open(options: MountOptions): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1200, height: 800 } });
    tab.setDefaultTimeout(8000);
    const errors: string[] = [];
    tab.on('pageerror', (error) => errors.push(String(error)));
    await tab.setContent(
      `<!doctype html><html><head><style>${PAGE_CSS}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    expect(errors, 'the bundle runs').toEqual([]);
    await tab.evaluate((last) => {
      const samples: Sample[] = [];
      (window as unknown as { samples: Sample[] }).samples = samples;
      const sample = (when: Sample['when']) => {
        const viewport = document.querySelector<HTMLElement>('.br-scroll-viewport');
        const rows = document.querySelectorAll('.row').length;
        if (!viewport || rows === 0) return;
        samples.push({
          rows,
          hasLast: document.getElementById(last) !== null,
          gap: Math.round(viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight),
          overflows: viewport.scrollHeight > viewport.clientHeight,
          when,
        });
      };
      new MutationObserver(() => sample('mutation')).observe(document.getElementById('root')!, {
        childList: true,
        subtree: true,
      });
      const frame = () => {
        sample('frame');
        requestAnimationFrame(frame);
      };
      requestAnimationFrame(frame);
    }, LAST);
    await tab.evaluate(
      (mount) => (window as unknown as { mount(o: MountOptions): void }).mount(mount),
      options
    );
    return tab;
  }

  /** Until the last batch has landed and the list has settled. */
  const settle = (tab: Page) =>
    tab.waitForFunction(
      (count) =>
        document.querySelectorAll('.row').length === count &&
        ((window as unknown as { settled?: number }).settled ?? 0) > 0,
      COUNT
    );

  const samples = (tab: Page) =>
    tab.evaluate(() => (window as unknown as { samples: Sample[] }).samples);

  /** A sample whose bottom is not the chat's last message, once the column overflows. */
  const offEnd = (all: Sample[]) => all.filter((s) => s.overflows && (!s.hasLast || s.gap > 1));

  it('the old order (oldest batch first) paints a bottom that is not the last message', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ opensAt: 'start', batchDelay: 20 });
    await settle(tab);
    const all = await samples(tab);
    expect(all.length).toBeGreaterThan(0);
    expect(all[0].hasLast, 'the instrument sees the first batch without the end').toBe(false);
    expect(offEnd(all).length, 'the instrument sees the walk to the end').toBeGreaterThan(0);
    await tab.close();
  });

  it('draws the newest batch first and never leaves the end while older ones load', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ batchDelay: 20 });
    await settle(tab);
    const all = await samples(tab);
    expect(all[0].rows, 'the first commit is one batch').toBe(20);
    expect(all.every((s) => s.hasLast)).toBe(true);
    expect(offEnd(all)).toEqual([]);
    expect(
      all.some((s) => s.when === 'mutation' && s.rows === 40),
      'a batch was added'
    ).toBe(true);
    await tab.close();
  });

  it('keeps the end when the newest batch fit in the viewport (no scroll to anchor on)', async (ctx) => {
    if (!browser) return ctx.skip();
    // The browser's own scroll anchoring keeps the place of a scrolled viewport
    // when rows are added above; at scrollTop 0 it does nothing, so a chat of
    // one-line messages whose newest batch fit would be left at its top.
    const tab = await open({ batchDelay: 20, short: true });
    await settle(tab);
    const all = await samples(tab);
    expect(all[0].overflows, 'the newest batch fit').toBe(false);
    expect(all[all.length - 1].overflows, 'the whole chat does not').toBe(true);
    expect(offEnd(all)).toEqual([]);
    await tab.close();
  });

  it('opens at the end when the messages arrive after the list mounted', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ arriveLater: true, batchDelay: 20 });
    await tab.evaluate(() => (window as unknown as { chat: { load(): void } }).chat.load());
    await settle(tab);
    const all = await samples(tab);
    expect(all[0].rows).toBe(20);
    expect(offEnd(all)).toEqual([]);
    await tab.close();
  });

  it('keeps a reader who scrolled up during the load on the rows they were reading', async (ctx) => {
    if (!browser) return ctx.skip();
    // A long delay, so the reader's scroll lands between two batches.
    const tab = await open({ batchDelay: 400 });
    await tab.waitForSelector(`#${LAST}`);
    const readTop = () =>
      tab.evaluate(() => {
        const viewport = document.querySelector<HTMLElement>('.br-scroll-viewport')!;
        const row = document.getElementById('row-45')!;
        return Math.round(row.getBoundingClientRect().top - viewport.getBoundingClientRect().top);
      });
    await tab.evaluate(() => {
      document.querySelector<HTMLElement>('.br-scroll-viewport')!.scrollTop -= 300;
    });
    await tab.waitForTimeout(50);
    const before = await readTop();
    await tab.waitForFunction(() => document.querySelectorAll('.row').length === 40);
    expect(await readTop(), 'the second batch moved nothing in view').toBe(before);
    await tab.waitForFunction(() => document.querySelectorAll('.row').length === 60);
    expect(await readTop(), 'the last batch moved nothing in view').toBe(before);
    await tab.close();
  }, 20_000);
});
