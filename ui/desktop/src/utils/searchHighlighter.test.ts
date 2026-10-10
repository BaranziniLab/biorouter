import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  clipMatchRect,
  glyphPaintedBeforeEllipsis,
  SearchHighlighter,
  trimRectToEllipsis,
  withoutDuplicateRects,
  type ClipBox,
} from './searchHighlighter';

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

/// Spec 3.16: the current match scrolls into view smoothly, but not under
/// reduced motion. jsdom lays nothing out, so the geometry is stubbed: a 200px
/// viewport over 2000px of content with a match at y = 1000. What a real engine
/// does with the scroll is pinned in `searchHighlighter.browser.test.ts`.
describe('SearchHighlighter scrolling to a match', () => {
  const MATCH_TOP = 1000;
  const MATCH_HEIGHT = 16;
  const VIEWPORT_HEIGHT = 200;
  /** The match is centred: its top minus the room left over either side of it. */
  const CENTRED = MATCH_TOP - (VIEWPORT_HEIGHT - MATCH_HEIGHT) / 2;

  const rect = (top: number, height: number) =>
    ({ top, height, bottom: top + height, left: 0, right: 100, width: 100 }) as DOMRect;

  const setUp = ({
    reducedMotion = false,
    scrollTo = true,
    scrollTop = 0,
    scrollHeight = 2000,
  }: {
    reducedMotion?: boolean;
    scrollTo?: boolean;
    scrollTop?: number;
    scrollHeight?: number;
  } = {}) => {
    vi.stubGlobal('ResizeObserver', ResizeObserverStub);
    vi.stubGlobal(
      'matchMedia',
      (query: string) => ({ matches: reducedMotion, media: query }) as MediaQueryList
    );
    document.body.innerHTML = `
      <div data-search-scroll-area>
        <div data-radix-scroll-area-viewport><div id="search-content">alpha beta</div></div>
      </div>`;
    const content = document.getElementById('search-content') as HTMLDivElement;
    const viewport = document.querySelector('[data-radix-scroll-area-viewport]') as HTMLDivElement;
    Object.defineProperty(viewport, 'clientHeight', { value: VIEWPORT_HEIGHT, configurable: true });
    Object.defineProperty(viewport, 'scrollHeight', { value: scrollHeight, configurable: true });
    viewport.getBoundingClientRect = () => rect(0, VIEWPORT_HEIGHT);
    viewport.scrollTop = scrollTop;
    // jsdom has no Element.scrollTo, so the smooth path is only taken where the test gives it one.
    const scrollToSpy = vi.fn();
    if (scrollTo) viewport.scrollTo = scrollToSpy as unknown as typeof viewport.scrollTo;

    const highlighter = new SearchHighlighter(content);
    // One measured match, standing where the layout engine would have put it.
    const mark = document.createElement('div');
    mark.className = 'search-highlight';
    mark.getBoundingClientRect = () => rect(MATCH_TOP, MATCH_HEIGHT);
    const container = document.createElement('div');
    container.appendChild(mark);
    const internals = highlighter as unknown as {
      highlights: HTMLElement[];
      currentTerm: string;
      isScrollingProgrammatically: boolean;
    };
    internals.highlights = [container];
    internals.currentTerm = 'alpha';
    // A scroll the hold lets through re-measures; count those instead of walking a document.
    const remeasure = vi.spyOn(highlighter, 'highlight').mockReturnValue([]);
    return { highlighter, viewport, scrollToSpy, remeasure, internals };
  };

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('scrolls smoothly to centre the match', () => {
    const { highlighter, viewport, scrollToSpy } = setUp();
    highlighter.setCurrentMatch(0, true);
    expect(scrollToSpy).toHaveBeenCalledWith({ top: CENTRED, behavior: 'smooth' });
    // Not jumped: the browser moves it, over time.
    expect(viewport.scrollTop).toBe(0);
  });

  it('jumps instead under reduced motion', () => {
    const { highlighter, viewport, scrollToSpy } = setUp({ reducedMotion: true });
    highlighter.setCurrentMatch(0, true);
    expect(scrollToSpy).not.toHaveBeenCalled();
    expect(viewport.scrollTop).toBe(CENTRED);
  });

  it('jumps where the element has no scrollTo', () => {
    const { highlighter, viewport } = setUp({ scrollTo: false });
    highlighter.setCurrentMatch(0, true);
    expect(viewport.scrollTop).toBe(CENTRED);
  });

  it('does not scroll when only the paint changed', () => {
    const { highlighter, viewport, scrollToSpy } = setUp();
    highlighter.setCurrentMatch(0, false);
    expect(scrollToSpy).not.toHaveBeenCalled();
    expect(viewport.scrollTop).toBe(0);
  });

  it('ignores its own scroll events until scrollend, then listens again', () => {
    const { highlighter, viewport, remeasure, internals } = setUp();
    highlighter.setCurrentMatch(0, true);
    expect(internals.isScrollingProgrammatically).toBe(true);

    // Well past the 100ms an instant scroll needs, still scrolling: still ignored.
    vi.advanceTimersByTime(600);
    viewport.dispatchEvent(new Event('scroll'));
    expect(remeasure).not.toHaveBeenCalled();

    viewport.dispatchEvent(new Event('scrollend'));
    expect(internals.isScrollingProgrammatically).toBe(false);
    viewport.dispatchEvent(new Event('scroll'));
    expect(remeasure).toHaveBeenCalledTimes(1);
  });

  it('stops ignoring scrolls after a second even if scrollend never comes', () => {
    const { highlighter, internals } = setUp();
    highlighter.setCurrentMatch(0, true);
    vi.advanceTimersByTime(999);
    expect(internals.isScrollingProgrammatically).toBe(true);
    vi.advanceTimersByTime(1);
    expect(internals.isScrollingProgrammatically).toBe(false);
  });

  /// A smooth scroll with nowhere to go fires neither scroll nor scrollend, so
  /// holding for the net would deafen the bar for a second for nothing.
  it('jumps, and holds only briefly, when the match is already as centred as the page allows', () => {
    // The page's last line: the clamp is 1800 and the container is already there.
    const { highlighter, viewport, scrollToSpy, internals } = setUp({ scrollTop: 1800 });
    highlighter.setCurrentMatch(0, true);
    expect(scrollToSpy).not.toHaveBeenCalled();
    expect(viewport.scrollTop).toBe(CENTRED + 1800);
    vi.advanceTimersByTime(100);
    expect(internals.isScrollingProgrammatically).toBe(false);
  });

  it('removes the pending scrollend listener when destroyed', () => {
    const { highlighter, viewport } = setUp();
    const removeEventListener = vi.spyOn(viewport, 'removeEventListener');
    highlighter.setCurrentMatch(0, true);
    highlighter.destroy();
    expect(removeEventListener).toHaveBeenCalledWith('scrollend', expect.any(Function));
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

/// The ellipsis rule, with the numbers the tester and the app measured. Whether
/// Chromium really paints this way is pinned against screen pixels in
/// `searchHighlighter.browser.test.ts`; this pins the arithmetic.
describe('trimRectToEllipsis', () => {
  // Settings → Skills, a 12px component line whose box ends at 1120: the "…" is
  // 12px wide, so the truncator keeps glyphs ending at or before 1108.
  const cut = { at: 1108 };
  const glyphsOf = (left: number, widths: number[]) =>
    widths.map((width, i) => {
      const start = left + widths.slice(0, i).reduce((sum, w) => sum + w, 0);
      return { left: start, right: start + width };
    });

  it('keeps a rect wholly before the cut as it is', () => {
    const rect = { left: 1004.1, top: 362, right: 1029.5, bottom: 377 };
    expect(trimRectToEllipsis(rect, [], cut)).toBe(rect);
  });

  /// "alignment-structural" at left 1114.9: its sliver inside the box is under
  /// the "…", so nothing of it is painted.
  it('drops a match that starts after the cut, even inside the box', () => {
    const rect = { left: 1114.9, top: 362, right: 1140.3, bottom: 377 };
    expect(trimRectToEllipsis(rect, glyphsOf(1114.9, [6.7, 2.7, 2.7, 6.7, 6.7]), cut)).toBeNull();
  });

  /// The hyphen of "receiving-code-review", wholly inside a box ending at 1084.
  /// A pixel clip at the cut would keep 1071 → 1072; the glyph is not painted.
  it('drops a glyph that straddles the cut rather than keeping its left part', () => {
    const hyphen = { left: 1071, top: 0, right: 1075, bottom: 14 };
    expect(trimRectToEllipsis(hyphen, [{ left: 1071, right: 1075 }], { at: 1072 })).toBeNull();
  });

  it('keeps a cut-through match up to the end of its last painted glyph', () => {
    const glyphs = glyphsOf(1090, [6, 6, 6, 6]); // rights 1096, 1102, 1108, 1114
    const rect = { left: 1090, top: 0, right: 1114, bottom: 14 };
    expect(trimRectToEllipsis(rect, glyphs, cut)).toEqual({ ...rect, right: 1108 });
  });

  it('keeps a glyph that ends exactly at the cut', () => {
    expect(glyphPaintedBeforeEllipsis({ left: 1100, right: 1108 }, cut)).toBe(true);
    expect(glyphPaintedBeforeEllipsis({ left: 1100, right: 1108.1 }, cut)).toBe(false);
  });
});

describe('withoutDuplicateRects', () => {
  /// A truncated line reports each painted match twice — the original fragment
  /// and the truncated copy — and both used to get a mark.
  it('keeps one of two rects a truncated line reports for the same glyphs', () => {
    const rect = { left: 548, top: 362, right: 573.4, bottom: 377 };
    expect(withoutDuplicateRects([rect, { ...rect, right: 573.45 }])).toEqual([rect]);
  });

  it('keeps the two lines of a match that wraps', () => {
    const first = { left: 300, top: 20, right: 330, bottom: 34 };
    const second = { left: 0, top: 36, right: 20, bottom: 50 };
    expect(withoutDuplicateRects([first, second])).toEqual([first, second]);
  });
});
