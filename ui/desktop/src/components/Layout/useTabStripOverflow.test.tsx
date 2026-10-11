import { act, render } from '@testing-library/react';
import { useRef } from 'react';
import { describe, expect, it } from 'vitest';
import { useTabStripScrolledEdges } from './useTabStripOverflow';

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
