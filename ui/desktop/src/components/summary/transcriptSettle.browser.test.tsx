// @vitest-environment node
/**
 * A reader who scrolls up while a long chat's older messages load keeps the
 * rows they were reading when the list settles (WS-TRANSCRIPT-1, the follow-up
 * to V2-R7-2).
 *
 * BaseChat's `handleRenderingComplete` used to land its FIRST settle on the end
 * whatever the reader had done. That settle is 50ms after the last batch lands,
 * and while the list opened at the top it was the moment the chat first reached
 * its end. Since V2-R7-2 a live chat opens at its end and adds older batches
 * above it, so the force only pulled a reader who had scrolled up during the
 * load (a few hundred ms on a long chat, seconds on one full of figures) back to
 * the end. `useTranscriptSettle` (what BaseChat now calls) leaves them.
 *
 * jsdom lays nothing out, so the real `ProgressiveMessageList`, `ScrollArea`
 * (with BaseChat's `autoScroll` and follow anchor), `useTranscriptFollow` and
 * `useTranscriptSettle` are bundled with esbuild and driven in Chromium, in a
 * frame shaped like BaseChat's column. The first case is the instrument: the
 * old handler, given the same steps, pulls the reader to the end.
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
import { useTranscriptFollow, useTranscriptSettle } from './useTranscriptFollow';

window.electron = { platform: 'darwin' };

const message = (i) => ({
  id: 'm' + i,
  role: i % 2 ? 'assistant' : 'user',
  created: 1700000000 + i,
  metadata: { userVisible: true, agentVisible: true },
  content: [{ type: 'text', text: 'message ' + i }],
});
const rowHeight = (index) => 40 + (index % 5) * 30;

// BaseChat's handler before this change, verbatim in what it decides.
function useLegacySettle(scrollRef, messageCount) {
  const initialRenderRef = React.useRef(true);
  return React.useCallback(() => {
    if (initialRenderRef.current && messageCount > 0) {
      initialRenderRef.current = false;
      if (scrollRef.current?.scrollToBottom) scrollRef.current.scrollToBottom('auto');
    } else if (scrollRef.current?.isFollowing) {
      if (scrollRef.current?.scrollToBottom) scrollRef.current.scrollToBottom();
    }
  }, [scrollRef, messageCount]);
}

function Frame({ legacy, count, batchDelay, arriveAfter }) {
  const all = React.useMemo(() => Array.from({ length: count }, (_, i) => message(i)), [count]);
  const scrollRef = React.useRef(null);
  const isStacked = React.useCallback(() => false, []);
  // The chat's messages arrive after it mounts, as BaseChat's load does.
  const [messages, setMessages] = React.useState([]);
  React.useEffect(() => {
    const timer = setTimeout(() => setMessages(all), arriveAfter);
    return () => clearTimeout(timer);
  }, [all, arriveAfter]);
  const follow = useTranscriptFollow({ scrollRef, isPreviewStacked: isStacked, transcriptMounted: true });
  const settle = useTranscriptSettle(scrollRef, messages.length);
  const legacySettle = useLegacySettle(scrollRef, messages.length);
  const onRenderingComplete = () => {
    window.settled = (window.settled || 0) + 1;
    (legacy ? legacySettle : settle)();
  };
  window.chat = { append: () => setMessages((m) => [...m, message(m.length)]) };
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

interface MountOptions {
  legacy?: boolean;
  count?: number;
  batchDelay?: number;
  /** When the chat's messages arrive after the list mounts (a load takes this long). */
  arriveAfter?: number;
}

describe('a reader who scrolled up during the load keeps their rows when it settles', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the transcript settle tests.');
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

  async function open({
    legacy = false,
    count = 60,
    batchDelay = 400,
    arriveAfter = 120,
  }: MountOptions) {
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
      { legacy, count, batchDelay, arriveAfter }
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

  /** A row's top edge in the viewport. */
  const rowTop = (tab: Page, index: number) =>
    tab.evaluate((i) => {
      const viewport = document.querySelector<HTMLElement>('.br-scroll-viewport')!;
      const row = document.getElementById(`row-${i}`)!;
      return Math.round(row.getBoundingClientRect().top - viewport.getBoundingClientRect().top);
    }, index);

  const scrollBy = (tab: Page, delta: number) =>
    tab.evaluate((d) => {
      document.querySelector<HTMLElement>('.br-scroll-viewport')!.scrollTop += d;
    }, delta);

  const toEnd = (tab: Page) =>
    tab.evaluate(() => {
      const viewport = document.querySelector<HTMLElement>('.br-scroll-viewport')!;
      viewport.scrollTop = viewport.scrollHeight;
    });

  /** Until the list has settled `times` times, then two frames and a beat for a scroll. */
  async function settled(tab: Page, rows: number, times = 1) {
    await tab.waitForFunction(
      ({ rows, times }) =>
        document.querySelectorAll('.row').length === rows &&
        ((window as unknown as { settled?: number }).settled ?? 0) >= times,
      { rows, times }
    );
    await tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );
    await tab.waitForTimeout(150);
  }

  it('the old handler pulls the reader back to the end at the first settle', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ legacy: true });
    await scrollBy(tab, -300);
    await tab.waitForTimeout(50);
    const before = await rowTop(tab, 45);
    await settled(tab, 60);
    expect(await gap(tab), 'the instrument lands on the end').toBeLessThanOrEqual(1);
    expect(await rowTop(tab, 45), 'the instrument moved the rows in view').not.toBe(before);
    await tab.close();
  }, 20_000);

  it('leaves the reader on the rows they were reading', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({});
    await scrollBy(tab, -300);
    await tab.waitForTimeout(50);
    const before = await rowTop(tab, 45);
    await settled(tab, 60);
    expect(await rowTop(tab, 45), 'the settle moved nothing in view').toBe(before);
    expect(await gap(tab)).toBeGreaterThan(200);
    // A message that arrives later does not pull them either.
    await tab.evaluate(() => (window as unknown as { chat: { append(): void } }).chat.append());
    await settled(tab, 61, 2);
    expect(await rowTop(tab, 45), 'a later message moved nothing in view').toBe(before);
    await tab.close();
  }, 20_000);

  it('leaves the reader also when the messages land inside the list’s first 50ms', async (ctx) => {
    if (!browser) return ctx.skip();
    // Messages that land inside the empty first render's 50ms settle timer.
    // Until eafea1b18 that timer fired with 20 of 60 rows drawn, so the first
    // settle was not the last; the reader is left either way.
    const tab = await open({ arriveAfter: 0 });
    await scrollBy(tab, -300);
    await tab.waitForTimeout(50);
    const before = await rowTop(tab, 45);
    await settled(tab, 60);
    expect(await rowTop(tab, 45), 'the settle moved nothing in view').toBe(before);
    expect(await gap(tab)).toBeGreaterThan(200);
    await tab.close();
  }, 20_000);

  it('still lands on the end when the reader did not scroll', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ batchDelay: 20 });
    await settled(tab, 60);
    expect(await gap(tab)).toBeLessThanOrEqual(1);
    await tab.close();
  });

  it('lands on the end when the reader scrolled up and came back before it settled', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({});
    await scrollBy(tab, -300);
    await tab.waitForTimeout(50);
    await toEnd(tab);
    await settled(tab, 60);
    expect(await gap(tab)).toBeLessThanOrEqual(1);
    await tab.close();
  }, 20_000);

  it('lands a short chat (no batches) on the end', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ count: 30 });
    await settled(tab, 30);
    expect(await gap(tab)).toBeLessThanOrEqual(1);
    await tab.close();
  });
});
