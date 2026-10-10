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
