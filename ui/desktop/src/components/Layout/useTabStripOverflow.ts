import { RefObject, useCallback, useEffect, useLayoutEffect, useState } from 'react';
import { shouldShowTabOverflowMenu } from './yieldLadder';

/**
 * Rung 3 of the yield ladder (D-32), as one rule for every tab strip.
 *
 * "Tabs shrink to a floor, then scroll, then collapse into a ▾ menu — never
 * wrap." The shrink and the scroll are CSS (`.br-tabstrip`); this is the last
 * step — whether the strip currently hides a tab, and therefore needs the menu.
 *
 * It lives here, and not inside ChatTabStrip, because the spec's card Z is
 * explicit that the chat strip and the preview panel are the SAME tab language.
 * They are still two components today, so a rule implemented in one of them is a
 * rule the other silently lacks — which is exactly what happened: the panel's
 * strip was `overflow-hidden`, so its tabs did not even scroll. They were simply
 * clipped and unreachable.
 *
 * @param ref the element with `overflow-x: auto` that holds the tabs
 * @param tabCount how many tabs it holds
 */
export function useTabStripOverflow(ref: RefObject<HTMLElement | null>, tabCount: number): boolean {
  const [overflowing, setOverflowing] = useState(false);

  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    setOverflowing(
      shouldShowTabOverflowMenu({
        scrollWidth: el.scrollWidth,
        clientWidth: el.clientWidth,
        tabCount,
      })
    );
  }, [ref, tabCount]);

  // Two triggers, because they are two different events and neither implies the
  // other: the OBSERVER catches the strip's box changing (window resize, sidebar
  // collapse, splitter drag, the preview panel taking its column), and the layout
  // effect catches the CONTENT changing (a tab opened, closed or renamed) — which
  // moves scrollWidth while leaving the observed box identical, so the observer
  // never fires for it.
  useLayoutEffect(() => {
    measure();
  });

  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref, measure]);

  return overflowing;
}

/** Which ends of a scrolling tab strip have tabs scrolled past them. */
export interface TabStripScrolledEdges {
  start: boolean;
  end: boolean;
}

const NOT_SCROLLED: TabStripScrolledEdges = { start: false, end: false };

/**
 * The scrolled-past ends of a tab strip, for the edge fade: a tab cut off at an
 * edge fades into it, so it reads as "more this way" rather than as a clipped
 * word beside the ▾ (vision V2-16 gave the preview panel's strip this; the chat
 * strip clipped "Variant c" mid-letter). A strip that fits reports neither end,
 * so it carries no mask at all.
 *
 * The same three triggers as {@link useTabStripOverflow}, plus the strip's own
 * scroll. State changes only when an end flips, so scrolling re-renders the strip
 * at most twice per pass.
 *
 * @param ref the element with `overflow-x: auto` that holds the tabs
 */
export function useTabStripScrolledEdges(
  ref: RefObject<HTMLElement | null>
): TabStripScrolledEdges {
  const [edges, setEdges] = useState<TabStripScrolledEdges>(NOT_SCROLLED);

  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    // 1px of slack: a fractional scroll width can leave scrollLeft a hair short
    // of its maximum, which must still read as "at the end".
    const start = el.scrollLeft > 1;
    const end = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
    setEdges((current) =>
      current.start === start && current.end === end ? current : { start, end }
    );
  }, [ref]);

  useLayoutEffect(() => {
    measure();
  });

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.addEventListener('scroll', measure, { passive: true });
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
    observer?.observe(el);
    return () => {
      el.removeEventListener('scroll', measure);
      observer?.disconnect();
    };
  }, [ref, measure]);

  return edges;
}
