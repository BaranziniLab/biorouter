import { act, render } from '@testing-library/react';
import { useRef } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  tabEdgeFadeOf,
  tabRevealScrollLeft,
  useTabStripOverflow,
  useTabStripScrolledEdges,
} from './useTabStripOverflow';

/**
 * The scrolled-past ends of a tab strip, which drive the strip's edge fade. jsdom
 * lays nothing out, so the scroll geometry is written onto the element the hook
 * reads; what is under test is the arithmetic and the triggers (a render, a
 * scroll), not the layout.
 */
function Strip({ onEdges }: { onEdges: (edges: { start: boolean; end: boolean }) => void }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const edges = useTabStripScrolledEdges(ref);
  onEdges(edges);
  return (
    <div
      ref={ref}
      data-testid="strip"
      data-scrolled-start={edges.start ? '' : undefined}
      data-scrolled-end={edges.end ? '' : undefined}
    />
  );
}

function setGeometry(
  el: HTMLElement,
  g: { scrollLeft: number; clientWidth: number; scrollWidth: number }
) {
  Object.defineProperty(el, 'clientWidth', { configurable: true, value: g.clientWidth });
  Object.defineProperty(el, 'scrollWidth', { configurable: true, value: g.scrollWidth });
  el.scrollLeft = g.scrollLeft;
}

describe('useTabStripScrolledEdges', () => {
  it('reports no end on a strip whose tabs fit, so it carries no mask', () => {
    let last = { start: true, end: true };
    const { getByTestId, rerender } = render(<Strip onEdges={(e) => (last = e)} />);
    setGeometry(getByTestId('strip'), { scrollLeft: 0, clientWidth: 600, scrollWidth: 600 });
    rerender(<Strip onEdges={(e) => (last = e)} />);
    expect(last).toEqual({ start: false, end: false });
    expect(getByTestId('strip')).not.toHaveAttribute('data-scrolled-end');
  });

  it('marks the end while tabs are hidden past it, both ends mid-scroll, and the start at the end', () => {
    let last = { start: false, end: false };
    const { getByTestId, rerender } = render(<Strip onEdges={(e) => (last = e)} />);
    const strip = getByTestId('strip');
    setGeometry(strip, { scrollLeft: 0, clientWidth: 600, scrollWidth: 1400 });
    rerender(<Strip onEdges={(e) => (last = e)} />);
    expect(last).toEqual({ start: false, end: true });

    act(() => {
      strip.scrollLeft = 400;
      strip.dispatchEvent(new Event('scroll'));
    });
    expect(last).toEqual({ start: true, end: true });

    act(() => {
      // A fractional scroll width can leave scrollLeft a hair short of its
      // maximum; that still reads as the end.
      strip.scrollLeft = 799.5;
      strip.dispatchEvent(new Event('scroll'));
    });
    expect(last).toEqual({ start: true, end: false });
    expect(strip).toHaveAttribute('data-scrolled-start');
    expect(strip).not.toHaveAttribute('data-scrolled-end');
  });
});

/**
 * The preview panel renders its tablist only once a tab is open, so the list mounts
 * AFTER the component that calls the hook (WS-PREVIEW-2). Both hooks used to attach
 * once, at their own mount, and so never watched such a list: scrolling it did not
 * re-render anything, and the edge flags went stale. They now follow the element the
 * ref holds as of the latest render.
 */
function LateStrip({
  show,
  listKey = 'a',
  onEdges,
  onOverflow,
}: {
  show: boolean;
  listKey?: string;
  onEdges?: (edges: { start: boolean; end: boolean }) => void;
  onOverflow?: (overflowing: boolean) => void;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  const edges = useTabStripScrolledEdges(ref);
  const overflowing = useTabStripOverflow(ref, show ? 9 : 0);
  onEdges?.(edges);
  onOverflow?.(overflowing);
  return (
    <div>{show ? <div key={listKey} ref={ref} data-testid={`strip-${listKey}`} /> : null}</div>
  );
}

describe('a tab strip list that mounts after the hook, or is replaced', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('follows the scroll of a list that mounted after the hook did', () => {
    let last = { start: false, end: false };
    const { getByTestId, rerender } = render(
      <LateStrip show={false} onEdges={(e) => (last = e)} />
    );
    rerender(<LateStrip show onEdges={(e) => (last = e)} />);
    const strip = getByTestId('strip-a');
    setGeometry(strip, { scrollLeft: 0, clientWidth: 600, scrollWidth: 1400 });
    // No render here: only the person scrolling, which is all that reaches a strip
    // that is not re-rendering.
    act(() => {
      strip.dispatchEvent(new Event('scroll'));
    });
    expect(last).toEqual({ start: false, end: true });
    act(() => {
      strip.scrollLeft = 400;
      strip.dispatchEvent(new Event('scroll'));
    });
    expect(last).toEqual({ start: true, end: true });
  });

  it('moves to the new element when the list is replaced, and lets the old one go', () => {
    let last = { start: false, end: false };
    const { getByTestId, rerender } = render(<LateStrip show onEdges={(e) => (last = e)} />);
    const first = getByTestId('strip-a');
    const removeFirst = vi.spyOn(first, 'removeEventListener');

    rerender(<LateStrip show listKey="b" onEdges={(e) => (last = e)} />);
    const second = getByTestId('strip-b');
    expect(removeFirst).toHaveBeenCalledWith('scroll', expect.any(Function));

    setGeometry(second, { scrollLeft: 0, clientWidth: 600, scrollWidth: 1400 });
    act(() => {
      second.scrollLeft = 400;
      second.dispatchEvent(new Event('scroll'));
    });
    expect(last).toEqual({ start: true, end: true });

    // The old element is gone from the page, but a scroll that reaches it anyway
    // must not move the flags.
    setGeometry(first, { scrollLeft: 0, clientWidth: 600, scrollWidth: 600 });
    act(() => {
      first.dispatchEvent(new Event('scroll'));
    });
    expect(last).toEqual({ start: true, end: true });
  });

  it('attaches once per element, not once per render, and detaches on unmount', () => {
    const { getByTestId, rerender, unmount } = render(<LateStrip show />);
    const strip = getByTestId('strip-a');
    const add = vi.spyOn(strip, 'addEventListener');
    const remove = vi.spyOn(strip, 'removeEventListener');
    rerender(<LateStrip show />);
    rerender(<LateStrip show />);
    expect(add).not.toHaveBeenCalled();
    expect(remove).not.toHaveBeenCalled();
    unmount();
    expect(remove).toHaveBeenCalledTimes(1);
    expect(remove).toHaveBeenCalledWith('scroll', expect.any(Function));
  });

  it('observes the size of a late or replaced list with both hooks, and disconnects on the way out', () => {
    const observed: Element[] = [];
    let disconnected = 0;
    class FakeObserver {
      observe(el: Element) {
        observed.push(el);
      }
      disconnect() {
        disconnected += 1;
      }
      unobserve() {}
    }
    vi.stubGlobal('ResizeObserver', FakeObserver);

    const { getByTestId, rerender, unmount } = render(<LateStrip show={false} />);
    expect(observed).toHaveLength(0);

    rerender(<LateStrip show />);
    const first = getByTestId('strip-a');
    // One observer from each hook.
    expect(observed.filter((el) => el === first)).toHaveLength(2);

    rerender(<LateStrip show />);
    expect(observed.filter((el) => el === first)).toHaveLength(2);

    rerender(<LateStrip show listKey="b" />);
    const second = getByTestId('strip-b');
    expect(observed.filter((el) => el === second)).toHaveLength(2);
    expect(disconnected).toBeGreaterThanOrEqual(2);

    const before = disconnected;
    unmount();
    expect(disconnected - before).toBe(2);
  });
});

/**
 * Where a reveal scrolls a strip (vision V2-R4-4). 15 tabs 136px wide every 144px from 8, the
 * strip 728px wide with 8px end padding, fading 32px at an end with tabs past it: the geometry
 * the finding measured (1100x760, the strip 172..900).
 */
describe('tabRevealScrollLeft', () => {
  const STRIP = { clientWidth: 728, scrollWidth: 8 + 14 * 144 + 136 + 8, fade: 32 };
  const at = (index: number, scrollLeft: number, over: Partial<typeof STRIP> = {}) =>
    tabRevealScrollLeft({
      ...STRIP,
      ...over,
      tabLeft: 8 + index * 144,
      tabWidth: 136,
      scrollLeft,
    });

  it('stops a tab straddling the end 32px short of it, outside the fade', () => {
    // Tab 5 is 728..864: the end is at 728.
    const left = at(5, 0)!;
    expect(left + STRIP.clientWidth - STRIP.fade).toBe(8 + 5 * 144 + 136);
  });

  it('stops a tab straddling the start 32px past it', () => {
    // Tab 9 is 1304..1440, the strip scrolled to 1340.
    expect(at(9, 1340)).toBe(8 + 9 * 144 - 32);
  });

  it('moves a tab inside the box but under a fade', () => {
    // Tab 5 is 728..864; at 150 the box is 150..878 and the end fade 846..878.
    expect(at(5, 150)).toBe(864 - 728 + 32);
    // At 168 its right edge meets the fade's inner edge: nothing to do.
    expect(at(5, 168)).toBeNull();
  });

  it('takes the last tab to the end and the first to the start, where nothing fades', () => {
    const max = STRIP.scrollWidth - STRIP.clientWidth;
    expect(at(14, 0)).toBe(max);
    expect(at(0, max)).toBe(0);
  });

  it('leaves a tab that is already clear where it is', () => {
    expect(at(2, 0)).toBeNull();
    expect(at(7, 600)).toBeNull();
  });

  it('a strip without a fade reveals to its edge', () => {
    expect(at(5, 0, { fade: 0 })).toBe(864 - 728);
  });

  it('a tab too wide to clear both fades lands whole, and stays put once shown', () => {
    // A 200px split strip whose tabs are at their cap (192px): no scroll clears both fades.
    const narrow = { clientWidth: 200, scrollWidth: 8 + 4 * 195 + 192 + 8, fade: 32 };
    const reveal = (index: number, scrollLeft: number) =>
      tabRevealScrollLeft({ ...narrow, tabLeft: 8 + index * 195, tabWidth: 192, scrollLeft });
    const middle = reveal(2, 0)!;
    expect(middle).toBeGreaterThanOrEqual(8 + 2 * 195 + 192 - 200);
    expect(middle).toBeLessThanOrEqual(8 + 2 * 195);
    expect(reveal(2, middle)).toBeNull();
    // The last tab: at the end only its start fades, so the end is the scroll that hides least.
    const max = narrow.scrollWidth - narrow.clientWidth;
    expect(reveal(4, 0)).toBe(max);
    expect(reveal(4, max)).toBeNull();
  });

  it('reads the fade from the strip, and 0 when it declares none', () => {
    const el = document.createElement('div');
    document.body.appendChild(el);
    expect(tabEdgeFadeOf(el)).toBe(0);
    el.style.setProperty('--tab-edge-fade', '32px');
    expect(tabEdgeFadeOf(el)).toBe(32);
    el.remove();
  });
});
