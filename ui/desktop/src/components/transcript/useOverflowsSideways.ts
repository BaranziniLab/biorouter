import { useLayoutEffect, useState, type CSSProperties } from 'react';

/**
 * The widest the right-edge fade ever is. The fade itself is
 * `--br-overflow-fade` (read by the `.br-md-*[data-overflow='true']` mask in
 * `main.css`): this, or what is still past the edge when that is less.
 */
export const OVERFLOW_FADE_MAX_PX = 40;

export interface SidewaysOverflow {
  overflows: boolean;
  more: boolean;
  fade: number;
}

/**
 * Whether a box is wider inside than it is shown, now and after any resize of
 * it or of its first child (the table or the code). Copied from Crew
 * (`crew/timeline/MessageBody.tsx`), not imported, so Crew stays untouched.
 *
 * `overflows`: it scrolls sideways at all, which is what makes it a region a
 * keyboard can reach. `more`: there is content past its right edge right now
 * (it overflows and is not scrolled to the end), measured again on every
 * scroll; it drives `data-overflow` and so the right-edge fade (`.br-md-*` in
 * `main.css`), because macOS hides overlay scrollbars. `fade`: how wide that
 * fade is, in whole pixels — what is still past the edge, at most
 * `OVERFLOW_FADE_MAX_PX`. A fixed 40px fade over a table only 10px too wide
 * dimmed a right-aligned last column almost to nothing (final check,
 * 2026-10-12); sized this way the fade never covers more of the view than is
 * hidden, and it narrows to nothing as the box reaches its end rather than
 * vanishing at once.
 *
 * Without a `ResizeObserver` (jsdom) it is measured once, and on each scroll.
 */
export function useOverflowsSideways<T extends HTMLElement>(): [
  (node: T | null) => void,
  SidewaysOverflow,
] {
  const [node, setNode] = useState<T | null>(null);
  const [state, setState] = useState<SidewaysOverflow>({
    overflows: false,
    more: false,
    fade: 0,
  });
  useLayoutEffect(() => {
    if (!node) return;
    const measure = () => {
      const overflows = node.scrollWidth > node.clientWidth + 1;
      const rest = node.scrollWidth - node.clientWidth - node.scrollLeft;
      const more = overflows && rest > 1;
      // Whole pixels, so a scroll re-renders at most once a pixel, and only in
      // the last 40px of travel.
      const fade = more ? Math.min(OVERFLOW_FADE_MAX_PX, Math.max(1, Math.round(rest))) : 0;
      setState((current) =>
        current.overflows === overflows && current.more === more && current.fade === fade
          ? current
          : { overflows, more, fade }
      );
    };
    measure();
    node.addEventListener('scroll', measure, { passive: true });
    if (typeof ResizeObserver !== 'function') {
      return () => node.removeEventListener('scroll', measure);
    }
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    if (node.firstElementChild) observer.observe(node.firstElementChild);
    return () => {
      observer.disconnect();
      node.removeEventListener('scroll', measure);
    };
  }, [node]);
  return [setNode, state];
}

/**
 * The attributes of a box that may scroll sideways: while it scrolls, a named
 * region a keyboard can reach; while there is more to its right,
 * `data-overflow` for the fade and `--br-overflow-fade` for its width. A box
 * that fits is neither, so it is no tab stop.
 */
export function scrollRegionProps(
  { overflows, more, fade }: SidewaysOverflow,
  name: string
): {
  role?: 'region';
  'aria-label'?: string;
  tabIndex?: number;
  'data-overflow'?: 'true';
  style?: CSSProperties;
} {
  return {
    ...(overflows ? { role: 'region' as const, 'aria-label': name, tabIndex: 0 } : {}),
    ...(more
      ? {
          'data-overflow': 'true' as const,
          style: { '--br-overflow-fade': `${fade}px` } as CSSProperties,
        }
      : {}),
  };
}
