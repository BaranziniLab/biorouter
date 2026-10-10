import { useLayoutEffect, useState } from 'react';

/**
 * Whether a box is wider inside than it is shown, now and after any resize of
 * it or of its first child (the table or the code). Copied from Crew
 * (`crew/timeline/MessageBody.tsx`), not imported, so Crew stays untouched.
 *
 * `overflows`: it scrolls sideways at all, which is what makes it a region a
 * keyboard can reach. `more`: there is content past its right edge right now
 * (it overflows and is not scrolled to the end), measured again on every
 * scroll; it drives `data-overflow` and so the right-edge fade (`.br-md-*` in
 * `main.css`), because macOS hides overlay scrollbars. At the end the fade
 * goes, so the last column is never dimmed.
 *
 * Without a `ResizeObserver` (jsdom) it is measured once.
 */
export function useOverflowsSideways<T extends HTMLElement>(): [
  (node: T | null) => void,
  { overflows: boolean; more: boolean },
] {
  const [node, setNode] = useState<T | null>(null);
  const [state, setState] = useState({ overflows: false, more: false });
  useLayoutEffect(() => {
    if (!node) return;
    const measure = () => {
      const overflows = node.scrollWidth > node.clientWidth + 1;
      const more = overflows && node.scrollLeft + node.clientWidth < node.scrollWidth - 1;
      setState((current) =>
        current.overflows === overflows && current.more === more ? current : { overflows, more }
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
 * `data-overflow` for the fade. A box that fits is neither, so it is no tab
 * stop.
 */
export function scrollRegionProps(
  { overflows, more }: { overflows: boolean; more: boolean },
  name: string
): {
  role?: 'region';
  'aria-label'?: string;
  tabIndex?: number;
  'data-overflow'?: 'true';
} {
  return {
    ...(overflows ? { role: 'region' as const, 'aria-label': name, tabIndex: 0 } : {}),
    ...(more ? { 'data-overflow': 'true' as const } : {}),
  };
}
