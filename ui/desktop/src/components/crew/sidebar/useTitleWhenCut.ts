import { useLayoutEffect, useRef } from 'react';

/**
 * A `title` holding `text` on the element exactly while it is cut short (SF-F6): in the 240px Crew
 * column a longer institution reads "stanf…", with nothing to read the rest by. The badge carries
 * no tooltip otherwise, since one reading "UCSF" over "UCSF" would only repeat it (Q2-17). Measured
 * as it lays out and again whenever its box resizes.
 */
export function useTitleWhenCut<T extends HTMLElement>(text: string | null) {
  const ref = useRef<T>(null);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const update = () => {
      if (text && element.scrollWidth > element.clientWidth) element.title = text;
      else element.removeAttribute('title');
    };
    update();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [text]);
  return ref;
}
