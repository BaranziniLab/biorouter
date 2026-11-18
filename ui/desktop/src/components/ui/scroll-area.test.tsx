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
function scriptViewport(
  viewport: HTMLElement,
  geometry: { height: number; content: number; width?: number }
) {
  const state = { width: 0, ...geometry, top: 0 };
  const max = () => Math.max(0, state.content - state.height);
  Object.defineProperty(viewport, 'clientHeight', { configurable: true, get: () => state.height });
  Object.defineProperty(viewport, 'clientWidth', { configurable: true, get: () => state.width });
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

  it('leaves a reader at the top alone after a resize that did not move it', () => {
    const { handle, viewport } = renderAnchored();
    scriptViewport(viewport, { height: 400, content: 1000, width: 500 });
    resize();

    act(() => viewport.dispatchEvent(new Event('scroll')));
    expect(handle.current?.isFollowing).toBe(false);
  });

  it('keeps following through native reflow scrolls before and after a width and height resize', () => {
    const { handle, viewport } = renderAnchored();
    const state = scriptViewport(viewport, { height: 400, content: 1000, width: 500 });
    resize();
    act(() => handle.current?.scrollToBottom('auto'));

    state.width = 350;
    state.height = 380;
    state.content = 1600;
    state.top = 800;
    act(() => viewport.dispatchEvent(new Event('scroll')));
    expect(handle.current?.isFollowing).toBe(true);
    resize();
    act(() => viewport.dispatchEvent(new Event('scroll')));
    expect(handle.current?.isFollowing).toBe(true);

    state.top = 400;
    act(() => viewport.dispatchEvent(new Event('scroll')));
    expect(handle.current?.isFollowing).toBe(false);
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
    return { state, grow, commit, viewport };
  }

  it('follows every growth to the end when no owner passes followGrowth', () => {
    const { state, grow } = renderFollowing();
    grow(62);
    runFrames();
    expect(state.top).toBe(62);
  });

  it('follows successive growth while its own previous follow is settling', () => {
    const { state, grow } = renderFollowing();
    grow(62);
    runFrames();
    grow(38);
    runFrames();
    expect(state.top).toBe(100);
  });

  it('keeps following when content grows before its own scroll event arrives', () => {
    const { state, grow, viewport } = renderFollowing();
    viewport.scrollTo = ((options: { top?: number }) => {
      viewport.scrollTop = options.top ?? 0;
    }) as typeof viewport.scrollTo;
    grow(62);
    runFrames();
    grow(432);
    act(() => viewport.dispatchEvent(new Event('scroll')));
    runFrames();
    expect(state.top).toBe(494);
  });

  it('stops following when the reader moves above the tail between updates', () => {
    const { state, grow, viewport } = renderFollowing();
    grow(600);
    runFrames();
    state.top = 250;
    act(() => viewport.dispatchEvent(new Event('scroll')));
    grow(62);
    runFrames();
    expect(state.top).toBe(250);
  });

  it('resumes growth after the reader returns to the tail and scrolling settles', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      const { state, grow, viewport } = renderFollowing();
      grow(600);
      runFrames();
      expect(state.top).toBe(600);
      state.top = 250;
      act(() => viewport.dispatchEvent(new Event('scroll')));
      act(() => vi.advanceTimersByTime(150));
      state.top = 600;
      act(() => viewport.dispatchEvent(new Event('scroll')));
      act(() => vi.advanceTimersByTime(150));
      grow(62);
      runFrames();
      expect(state.top).toBe(662);
    } finally {
      vi.useRealTimers();
    }
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

  /**
   * The 2026-10-12 defect: `scrollIntoView` scrolled the `overflow: hidden` root along with the
   * viewport, the wheel returned only the viewport to 0, and the page's first lines stayed under
   * the band. Chromium is where the root stops having anything to scroll
   * (`settings/pageHeadroom.browser.test.tsx`); this is the component's own promise that a root
   * which does scroll is put back.
   */
  it('puts its root back at the origin whenever anything scrolls it', () => {
    const { container } = render(
      <ScrollArea>
        <div />
      </ScrollArea>
    );
    const root = container.querySelector<HTMLElement>('.br-scroll-root')!;
    const offset = { top: 0, left: 0 };
    Object.defineProperty(root, 'scrollTop', {
      configurable: true,
      get: () => offset.top,
      set: (value: number) => {
        offset.top = value;
      },
    });
    Object.defineProperty(root, 'scrollLeft', {
      configurable: true,
      get: () => offset.left,
      set: (value: number) => {
        offset.left = value;
      },
    });
    offset.top = 435;
    offset.left = 12;
    act(() => {
      root.dispatchEvent(new Event('scroll'));
    });
    expect(offset).toEqual({ top: 0, left: 0 });
  });

  it('counts a whole pixel as scrolled, so a fractional rest at the top keeps no fade', () => {
    const { container } = render(
      <ScrollArea>
        <div />
      </ScrollArea>
    );
    const root = container.querySelector('.br-scroll-root')!;
    const viewport = container.querySelector<HTMLElement>('.br-scroll-viewport')!;
    const state = scriptViewport(viewport, { height: 100, content: 1000 });
    const scrollTo = (top: number) =>
      act(() => {
        state.top = top;
        viewport.dispatchEvent(new Event('scroll'));
      });
    scrollTo(40);
    expect(root).toHaveAttribute('data-scrolled', 'true');
    scrollTo(0.5);
    expect(root).toHaveAttribute('data-scrolled', 'false');
    scrollTo(1);
    expect(root).toHaveAttribute('data-scrolled', 'true');
    scrollTo(0);
    expect(root).toHaveAttribute('data-scrolled', 'false');
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
