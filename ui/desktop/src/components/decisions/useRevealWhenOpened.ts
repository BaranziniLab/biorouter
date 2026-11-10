import { useEffect, type RefObject } from 'react';

/** Longer than the disclosure panel's open animation (`--dur-med`), as a fallback. */
export const REVEAL_FALLBACK_MS = 400;

/**
 * G-7: a transcript row opened at the end of the conversation grew BELOW the scroll
 * viewport, so its well and its "Decision history" link sat hidden under the composer
 * bar until the person scrolled. The transcript's `ScrollArea` follows growth only when
 * its own children re-render (a new message), and a row's disclosure is the row's own
 * state, so that growth was never followed.
 *
 * Mount this in a disclosure's BODY (Radix unmounts it while closed, so mounting is the
 * person opening it): once the panel's open animation ends, the element is scrolled into
 * view with `block: 'nearest'`, which moves the nearest scroll container only as far as
 * needed and not at all when it is already visible. Reduced motion jumps.
 *
 * ⚠ A local fix for the rows that carry it (the turn marker's well and the `decide`
 * card). The general one (every other tool row has the same growth) belongs to the
 * transcript's scroll follow in `ui/scroll-area.tsx`; once that follows growth by a
 * ResizeObserver, this hook is redundant and should go.
 */
export function useRevealWhenOpened(ref: RefObject<HTMLElement | null>) {
  useEffect(() => {
    const element = ref.current;
    if (!element || typeof element.scrollIntoView !== 'function') return;
    const panel = element.closest('.biorouter-disclosure-panel');
    let done = false;
    const reveal = () => {
      if (done || !element.isConnected) return;
      done = true;
      const reduced =
        typeof window.matchMedia === 'function' &&
        window.matchMedia('(prefers-reduced-motion: reduce)').matches;
      element.scrollIntoView({ block: 'nearest', behavior: reduced ? 'auto' : 'smooth' });
    };
    const onEnd = (event: Event) => {
      if (event.target === panel) reveal();
    };
    panel?.addEventListener('animationend', onEnd);
    const timer = setTimeout(reveal, REVEAL_FALLBACK_MS);
    return () => {
      done = true;
      clearTimeout(timer);
      panel?.removeEventListener('animationend', onEnd);
    };
    // Once per open: the body remounts on the next one.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
