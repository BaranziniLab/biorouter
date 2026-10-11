import { act, render } from '@testing-library/react';
import { useRef } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useTabStripOverflow, useTabStripScrolledEdges } from './useTabStripOverflow';

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
