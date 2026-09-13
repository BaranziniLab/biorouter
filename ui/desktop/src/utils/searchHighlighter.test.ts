import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { clipMatchRect, SearchHighlighter, type ClipBox } from './searchHighlighter';

class ResizeObserverStub {
  observe = vi.fn();
  disconnect = vi.fn();
}

describe('SearchHighlighter', () => {
  beforeEach(() => {
    vi.stubGlobal('ResizeObserver', ResizeObserverStub);
    document.body.innerHTML = `
      <div data-search-scroll-area>
        <div data-radix-scroll-area-viewport>
          <div id="search-content">alpha beta</div>
        </div>
      </div>
    `;
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('removes its scroll listener when destroyed', () => {
    const content = document.getElementById('search-content') as HTMLDivElement;
    const viewport = document.querySelector('[data-radix-scroll-area-viewport]') as HTMLDivElement;
    const addEventListener = vi.spyOn(viewport, 'addEventListener');
    const removeEventListener = vi.spyOn(viewport, 'removeEventListener');

    const highlighter = new SearchHighlighter(content);
    const scrollRegistration = addEventListener.mock.calls.find(([type]) => type === 'scroll');
    expect(scrollRegistration).toBeDefined();
    // Capture phase, so an inner scroller's scroll (which does not bubble)
    // re-measures the marks too.
    expect(scrollRegistration?.[2]).toMatchObject({ capture: true });

    highlighter.destroy();

    // A capture listener is only removed by a removal that also says capture.
    expect(removeEventListener).toHaveBeenCalledWith('scroll', scrollRegistration?.[1], {
      capture: true,
    });
  });
});

/// The rule itself, free of layout: which rects count and what gets painted.
/// Where the rule meets real CSS (line-clamp, overflow, scrolling) is pinned in
/// `searchHighlighter.browser.test.ts`, because jsdom lays nothing out.
describe('clipMatchRect', () => {
  const line = { left: 100, top: 20, right: 160, bottom: 34 }; // a 14px-tall word

  const hides = (box: Partial<ClipBox>): ClipBox => ({
    left: 0,
    top: 0,
    right: 400,
    bottom: 400,
    x: 'hides',
    y: 'hides',
    ...box,
  });

  it('counts and paints a rect no clip touches', () => {
    expect(clipMatchRect(line, [])).toEqual({ counts: true, paint: line });
  });

  it('does not count a rect wholly below a line clamp', () => {
    // The Settings → Skills measurement: the clamp's visible line ends at 438,
    // the next line's word starts at 439.
    const clamp = hides({ top: 422, bottom: 438 });
    expect(clipMatchRect({ left: 10, top: 439, right: 20, bottom: 453 }, [clamp])).toEqual({
      counts: false,
      paint: null,
    });
  });

  it('counts a rect cut through but mostly shown, and paints only the shown part', () => {
    const box = hides({ bottom: 30 }); // 10 of 14px shown
    expect(clipMatchRect(line, [box])).toEqual({
      counts: true,
      paint: { left: 100, top: 20, right: 160, bottom: 30 },
    });
  });

  it('does not count a sliver — the overhang of the next line into a clamp', () => {
    const box = hides({ bottom: 23 }); // 3 of 14px shown
    expect(clipMatchRect(line, [box]).counts).toBe(false);
  });

  it('counts a word a truncated line cuts sideways while any of it shows', () => {
    const box = hides({ right: 110 });
    expect(clipMatchRect(line, [box])).toEqual({
      counts: true,
      paint: { left: 100, top: 20, right: 110, bottom: 34 },
    });
  });

  it('only clips on the axes a box clips', () => {
    // `overflow-x: hidden` with a visible y: a rect below the box still shows.
    const box = hides({ bottom: 10, y: 'visible' });
    expect(clipMatchRect(line, [box])).toEqual({ counts: true, paint: line });
  });

  it('counts a rect a scroller has out of view, and paints nothing for it', () => {
    const scroller: ClipBox = {
      left: 0,
      top: 0,
      right: 80,
      bottom: 400,
      x: 'scrolls',
      y: 'scrolls',
    };
    expect(clipMatchRect(line, [scroller])).toEqual({ counts: true, paint: null });
  });

  it('paints the part of a rect a scroller shows', () => {
    const scroller: ClipBox = {
      left: 0,
      top: 0,
      right: 130,
      bottom: 400,
      x: 'scrolls',
      y: 'visible',
    };
    expect(clipMatchRect(line, [scroller])).toEqual({
      counts: true,
      paint: { left: 100, top: 20, right: 130, bottom: 34 },
    });
  });

  it('lets a hiding clip decide the count even inside a scroller', () => {
    const scroller: ClipBox = {
      left: 0,
      top: 0,
      right: 400,
      bottom: 400,
      x: 'scrolls',
      y: 'scrolls',
    };
    const clamp = hides({ bottom: 18 });
    expect(clipMatchRect(line, [clamp, scroller]).counts).toBe(false);
  });

  it('never counts an empty rect', () => {
    expect(clipMatchRect({ left: 5, top: 5, right: 5, bottom: 19 }, []).counts).toBe(false);
  });
});
