import { RefObject, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { shouldShowTabOverflowMenu } from './yieldLadder';

/**
 * Keeps a ResizeObserver (and, for the edge fade, a `scroll` listener) on whatever
 * element `ref` holds NOW, and moves them when it holds a different one.
 *
 * Both hooks below used to attach in `useEffect(..., [ref, measure])`. A ref object
 * is identity-stable, so that effect ran once, at the hook's mount: a list that was
 * not in the DOM yet (the preview panel renders its tablist only once a tab is
 * open) was never watched, and a list that unmounted and came back was watched at
 * its old, detached element. Scrolling does not re-render, so the edge flags then
 * went stale while the person scrolled (WS-PREVIEW-2).
 *
 * The attach rides the layout effect that already runs after every render and
 * re-measures, so a list that mounts in a render of the component calling the hook
 * is picked up in that same commit. It compares the element and `measure`, so a
 * render that changes neither does nothing. The detach is on unmount only.
 */
function useWatchedElement(
  ref: RefObject<HTMLElement | null>,
  measure: () => void,
  listenToScroll: boolean
) {
  const attached = useRef<{ el: HTMLElement; measure: () => void; detach: () => void } | null>(
    null
  );

  useLayoutEffect(() => {
    const el = ref.current;
    const current = attached.current;
    if (current && current.el === el && current.measure === measure) return;
    current?.detach();
    attached.current = null;
    if (!el) return;
    if (listenToScroll) el.addEventListener('scroll', measure, { passive: true });
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
    observer?.observe(el);
    attached.current = {
      el,
      measure,
      detach: () => {
        if (listenToScroll) el.removeEventListener('scroll', measure);
        observer?.disconnect();
      },
    };
  });

  useEffect(
    () => () => {
      attached.current?.detach();
      attached.current = null;
    },
    []
  );
}

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
 * @param ref the element with `overflow-x: auto` that holds the tabs. It may mount
 *   after the hook does, or be replaced by another element; it is watched wherever
 *   it is as of the latest render.
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

  useWatchedElement(ref, measure, false);

  return overflowing;
}

/** Which ends of a scrolling tab strip have tabs scrolled past them. */
export interface TabStripScrolledEdges {
  start: boolean;
  end: boolean;
}

const NOT_SCROLLED: TabStripScrolledEdges = { start: false, end: false };

/**
 * Which ends of a strip would have tabs scrolled past them at `scrollLeft`. The
 * one definition of "this end fades": the edge-fade hook below reads it from the
 * strip as it is, and {@link tabRevealScrollLeft} asks it of a scroll it has not
 * made yet, so a reveal and the fade can never disagree about which ends fade.
 *
 * 1px of slack: a fractional scroll width can leave scrollLeft a hair short of its
 * maximum, which must still read as "at the end".
 */
export function scrolledEdgesAt(
  scrollLeft: number,
  clientWidth: number,
  scrollWidth: number
): TabStripScrolledEdges {
  return { start: scrollLeft > 1, end: scrollLeft + clientWidth < scrollWidth - 1 };
}

/**
 * The strip's edge fade in px: its `--tab-edge-fade` (the chat strip's and the
 * preview panel's 32px), or 0 for a strip that declares none.
 */
export function tabEdgeFadeOf(strip: HTMLElement): number {
  const fade = parseFloat(getComputedStyle(strip).getPropertyValue('--tab-edge-fade'));
  return Number.isFinite(fade) && fade > 0 ? fade : 0;
}

/** What {@link tabRevealScrollLeft} reads: one tab and the strip that scrolls it. */
export interface TabRevealGeometry {
  /** The tab's start and width in the strip's scroll coordinates (`offsetLeft`, `offsetWidth`). */
  tabLeft: number;
  tabWidth: number;
  scrollLeft: number;
  clientWidth: number;
  scrollWidth: number;
  /** The strip's edge fade ({@link tabEdgeFadeOf}). */
  fade: number;
}

/**
 * Where a strip should scroll so a tab is seen whole, or null when it already is.
 *
 * "Whole" means clear of the edge fade as well as inside the box: an end with
 * tabs scrolled past it fades over `fade` px, so a tab revealed only to the box's
 * edge ends under the fade, its close slot and the end of its accent strip faded
 * out (vision V2-R4-4: revealed to 8px from the end of a strip whose end fades
 * over 32). A reveal therefore puts the tab's near end at the fade's inner edge.
 * A reveal that reaches the end of the strip turns that end's fade off, and the
 * tab then sits at the end; {@link scrolledEdgesAt} says which ends fade at each
 * candidate scroll.
 *
 * A tab too wide to clear both fades (a narrow split pane) still lands whole
 * inside the box, with as little of it under a fade as the strip allows. Ties go
 * to the smaller move, and a tab already shown as well as it can be is left where
 * it is: revealing the same tab twice never moves the strip twice.
 */
export function tabRevealScrollLeft(g: TabRevealGeometry): number | null {
  const max = Math.max(0, g.scrollWidth - g.clientWidth);
  const clamp = (left: number) => Math.min(max, Math.max(0, left));
  const tabRight = g.tabLeft + g.tabWidth;
  const shown = (from: number, to: number) =>
    Math.max(0, Math.min(tabRight, to) - Math.max(g.tabLeft, from));
  // At scroll `s`: how much of the tab is out of the box, and how much of what
  // is in it sits under a fade.
  const costAt = (s: number) => {
    const edges = scrolledEdgesAt(s, g.clientWidth, g.scrollWidth);
    const inBox = shown(s, s + g.clientWidth);
    const clear = shown(
      s + (edges.start ? g.fade : 0),
      s + g.clientWidth - (edges.end ? g.fade : 0)
    );
    return { s, outside: g.tabWidth - inBox, faded: inBox - clear };
  };
  type Cost = ReturnType<typeof costAt>;
  // Out of the box is worse than faded (a faded close slot can still be seen
  // and clicked), then less faded, then the smaller move. Half a pixel of slack
  // for the rounding in offsetLeft and offsetWidth.
  const better = (a: Cost, b: Cost) => {
    if (Math.abs(a.outside - b.outside) > 0.5) return a.outside < b.outside;
    if (Math.abs(a.faded - b.faded) > 0.5) return a.faded < b.faded;
    return Math.abs(a.s - g.scrollLeft) < Math.abs(b.s - g.scrollLeft);
  };
  const here = costAt(g.scrollLeft);
  if (here.outside <= 0.5 && here.faded <= 0.5) return null;
  let best = here;
  for (const left of [
    clamp(g.tabLeft - g.fade),
    clamp(tabRight - g.clientWidth + g.fade),
    clamp(g.tabLeft),
    clamp(tabRight - g.clientWidth),
  ]) {
    const cost = costAt(left);
    if (better(cost, best)) best = cost;
  }
  return best === here || best.s === g.scrollLeft ? null : best.s;
}

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
 * @param ref the element with `overflow-x: auto` that holds the tabs. It may mount
 *   after the hook does (the preview panel renders its list once a tab is open) or
 *   be replaced by another element; the listener follows it on the render that
 *   changes it, so the component calling the hook must be the one that renders it.
 */
export function useTabStripScrolledEdges(
  ref: RefObject<HTMLElement | null>
): TabStripScrolledEdges {
  const [edges, setEdges] = useState<TabStripScrolledEdges>(NOT_SCROLLED);

  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const { start, end } = scrolledEdgesAt(el.scrollLeft, el.clientWidth, el.scrollWidth);
    setEdges((current) =>
      current.start === start && current.end === end ? current : { start, end }
    );
  }, [ref]);

  useLayoutEffect(() => {
    measure();
  });

  useWatchedElement(ref, measure, true);

  return edges;
}
