import { useLayoutEffect, useState, type RefObject } from 'react';
import { homeUsageFit, type HomeUsageFit, type HomeUsageMeasure } from './homeUsageFit';

/**
 * Reads `homeUsageFit`'s inputs off Home's insights block and keeps the answer.
 *
 * `root` is `.biorouter-home-insights`; its parent is the scroller above the composer. The card
 * sits in `.biorouter-home-usage`, and its heat map and footnote in `.br-usage-card-extra`.
 * Both of those fold away by clipping (`main.css`), so the card and the heat map keep their
 * natural size inside and are measured in every state. `card` names the card that is mounted
 * (the loading card or the loaded one), so a swap re-attaches the observer to the new box.
 */
export function useHomeUsageFit(
  root: RefObject<HTMLElement | null>,
  card: 'loading' | 'loaded' | null
): HomeUsageFit {
  const [fit, setFit] = useState<HomeUsageFit>('full');

  useLayoutEffect(() => {
    const insights = root.current;
    if (!card || !insights) return;
    const scroller = insights.parentElement;
    const column = insights.querySelector<HTMLElement>('.biorouter-home-column');
    const usage = insights.querySelector<HTMLElement>('.biorouter-home-usage');
    const surface = usage?.querySelector<HTMLElement>('.br-usage-card');
    const extra = surface?.querySelector<HTMLElement>('.br-usage-card-extra');
    const extraBody = extra?.firstElementChild as HTMLElement | null | undefined;
    if (!scroller || !column || !usage || !surface) return;

    const height = (el: Element | null | undefined) => el?.getBoundingClientRect().height ?? 0;
    const measure = () => {
      const shown = height(extra);
      const natural = height(extraBody);
      const cardHeight = height(surface);
      const m: HomeUsageMeasure = {
        columnWidth: column.getBoundingClientRect().width,
        roomHeight: scroller.clientHeight,
        restHeight: height(column) - height(usage),
        fullHeight: cardHeight - shown + natural,
        compactHeight: cardHeight - shown,
      };
      const next = homeUsageFit(m);
      setFit((previous) => (previous === next ? previous : next));
    };

    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    for (const el of [scroller, column, surface, extraBody]) if (el) observer.observe(el);
    return () => observer.disconnect();
  }, [root, card]);

  return fit;
}
