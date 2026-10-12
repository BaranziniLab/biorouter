import { act, render } from '@testing-library/react';
import { createRef } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ScrollArea, type ScrollAreaHandle } from './scroll-area';

/**
 * The resize anchor (`anchorBottomOnResize`) keeps the transcript's bottom edge in place when the
 * viewport changes height. jsdom has no layout, so the viewport's geometry is scripted here and the
 * resize observer is fired by hand, in the order a browser uses: layout changes the height, scroll
 * events for that frame are dispatched, then the observer runs.
 */

type ObserverCallback = (entries: unknown[], observer: unknown) => void;

let observers: ObserverCallback[] = [];

class ScriptedResizeObserver {
  constructor(private readonly callback: ObserverCallback) {
    observers.push(callback);
  }
  observe() {}
  unobserve() {}
  disconnect() {
    observers = observers.filter((callback) => callback !== this.callback);
  }
}

function resize() {
  act(() => {
    for (const callback of [...observers]) callback([], {});
  });
}

/** A viewport whose height, content height and scroll position the test controls. */
function scriptViewport(viewport: HTMLElement, geometry: { height: number; content: number }) {
  const state = { ...geometry, top: 0 };
  const max = () => Math.max(0, state.content - state.height);
  Object.defineProperty(viewport, 'clientHeight', { configurable: true, get: () => state.height });
  Object.defineProperty(viewport, 'scrollHeight', { configurable: true, get: () => state.content });
  Object.defineProperty(viewport, 'scrollTop', {
    configurable: true,
    get: () => state.top,
    set: (value: number) => {
      state.top = Math.min(Math.max(0, value), max());
    },
  });
  viewport.scrollTo = ((options: { top?: number }) => {
    state.top = Math.min(Math.max(0, options.top ?? 0), max());
    viewport.dispatchEvent(new Event('scroll'));
  }) as typeof viewport.scrollTo;
  return state;
}

function renderAnchored() {
  const handle = createRef<ScrollAreaHandle>();
  render(
    <ScrollArea ref={handle} autoScroll anchorBottomOnResize={() => true}>
      <div />
    </ScrollArea>
  );
  const viewport = handle.current?.viewportRef.current;
  if (!viewport) throw new Error('No viewport.');
  return { handle, viewport };
}

describe('ScrollArea resize anchor', () => {
  beforeEach(() => {
    observers = [];
    vi.stubGlobal('ResizeObserver', ScriptedResizeObserver);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('keeps the reader’s bottom line against the bottom when the viewport shrinks', () => {
    const { viewport } = renderAnchored();
    const state = scriptViewport(viewport, { height: 400, content: 1000 });
    resize();

    // The reader scrolls to the middle: the line at 700 sits against the bottom edge.
    state.top = 300;
    act(() => {
      viewport.dispatchEvent(new Event('scroll'));
    });
    state.height = 300;
    resize();
    expect(state.top).toBe(400);
  });

  it('lands at the end when the owner scrolls to the bottom while a resize is still pending', () => {
    const { handle, viewport } = renderAnchored();
    const state = scriptViewport(viewport, { height: 348, content: 713 });
    // The anchor mounted at the top: it remembers a bottom edge of 348.
    resize();

    // A note above the composer grows in the same frame the channel opens at its newest message:
    // the height is new, the observer has not run, and the scroll arrives in between.
    state.height = 336;
    act(() => handle.current?.scrollToBottom('auto'));
    expect(state.top).toBe(713 - 336);

    resize();
    expect(state.top).toBe(713 - 336);
  });
});

/**
 * The growth follow (`autoScroll`) and its owner's veto (`followGrowth`, vision V2-R9-2): while
 * the reader follows the end, content that grows is followed there in the next animation frame,
 * unless the owner holds that growth below the fold. Frames are queued and run by hand, so a
 * growth's commit and its frame can be told apart.
 */
type Frame = (time: number) => void;

describe('ScrollArea growth follow', () => {
  let frames: Frame[] = [];

  function runFrames() {
    act(() => {
      const queued = frames;
      frames = [];
      for (const frame of queued) frame(0);
    });
  }

  beforeEach(() => {
    frames = [];
    vi.stubGlobal('requestAnimationFrame', (callback: Frame) => {
      frames.push(callback);
      return frames.length;
    });
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  /** A following transcript of 400px in a 400px viewport; `grow` adds content and commits. */
  function renderFollowing(followGrowth?: () => boolean) {
    const handle = createRef<ScrollAreaHandle>();
    const view = (rows: number) => (
      <ScrollArea ref={handle} autoScroll followGrowth={followGrowth}>
        <div>{rows}</div>
      </ScrollArea>
    );
    const { rerender } = render(view(0));
    const viewport = handle.current?.viewportRef.current;
    if (!viewport) throw new Error('No viewport.');
    const state = scriptViewport(viewport, { height: 400, content: 400 });
    let rows = 1;
    // The first commit with geometry is the transcript being drawn: it grows from nothing.
    rerender(view(rows));
    runFrames();
    const grow = (by: number) => {
      state.content += by;
      rows += 1;
      rerender(view(rows));
    };
    const commit = () => rerender(view(rows));
    return { state, grow, commit };
  }

  it('follows every growth to the end when no owner passes followGrowth', () => {
    const { state, grow } = renderFollowing();
    grow(62);
    runFrames();
    expect(state.top).toBe(62);
  });

  it('keeps scrollTop for a growth the owner holds, and does not follow it on a later commit', () => {
    let follow = true;
    const { state, grow, commit } = renderFollowing(() => follow);
    follow = false;
    grow(62);
    runFrames();
    expect(state.top).toBe(0);

    // The hold is released, but the held row is old news: nothing grew since.
    follow = true;
    commit();
    runFrames();
    expect(state.top).toBe(0);

    // The next growth is followed, to the end that now includes the held row.
    grow(38);
    runFrames();
    expect(state.top).toBe(100);
  });

  it('decides at the growth: one held at its commit stays put when the hold ends before the frame', () => {
    let follow = true;
    const { state, grow } = renderFollowing(() => follow);
    follow = false;
    grow(62);
    follow = true;
    runFrames();
    expect(state.top).toBe(0);
  });

  it('asks again in the frame: a growth whose frame has not run when the hold starts stays put', () => {
    let follow = true;
    const { state, grow } = renderFollowing(() => follow);
    grow(62);
    follow = false;
    runFrames();
    expect(state.top).toBe(0);
  });
});

/**
 * Spec 2.6: the scrollbar rests hidden and shows on hover and while scrolling. The transcript's
 * native thumb used to be visible all the time (the global rule forces the native scrollbar on,
 * and `scrollbar-color` paints it). Hover is CSS; "while scrolling" is a root attribute.
 */
describe('ScrollArea scrollbar visibility', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('marks the root while scrolling and clears the mark once scrolling settles', async () => {
    vi.useFakeTimers();
    const { SCROLLING_SETTLE_MS } = await import('./scroll-area');
    const { container } = render(
      <ScrollArea>
        <div />
      </ScrollArea>
    );
    const root = container.querySelector('.br-scroll-root')!;
    const viewport = container.querySelector('.br-scroll-viewport')!;
    expect(root).not.toHaveAttribute('data-scrolling');

    act(() => {
      viewport.dispatchEvent(new Event('scroll'));
    });
    expect(root).toHaveAttribute('data-scrolling');

    act(() => {
      vi.advanceTimersByTime(SCROLLING_SETTLE_MS + 1);
    });
    expect(root).not.toHaveAttribute('data-scrolling');
  });

  it('hides the thumb at rest and shows it on hover or while scrolling, in main.css', async () => {
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    const css = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');
    const body = (selector: string) => {
      const at = css.indexOf(`\n${selector} {`);
      expect(at, `no rule for ${selector}`).toBeGreaterThanOrEqual(0);
      return css.slice(css.indexOf('{', at) + 1, css.indexOf('}', at));
    };
    expect(body('.br-scroll-viewport')).toMatch(/scrollbar-color:\s*transparent transparent/);
    expect(body('.br-scroll-root:is(:hover, [data-scrolling]) > .br-scroll-viewport')).toMatch(
      /scrollbar-color:\s*var\(--border-strong\) transparent/
    );
  });
});
