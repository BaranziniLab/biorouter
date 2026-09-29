import {
  forwardRef,
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type HTMLAttributes,
} from 'react';
import { middleTruncate } from './middleName';

let measuringContext: CanvasRenderingContext2D | null | undefined;

/** A canvas context to measure text with, or null where there is none. */
function measuring(): CanvasRenderingContext2D | null {
  if (measuringContext !== undefined) return measuringContext;
  try {
    const canvas = document.createElement('canvas');
    measuringContext = typeof canvas.getContext === 'function' ? canvas.getContext('2d') : null;
  } catch {
    measuringContext = null;
  }
  return measuringContext;
}

export interface MiddleTruncatedNameProps extends HTMLAttributes<HTMLSpanElement> {
  name: string;
  className: string;
}

/**
 * A file name drawn in `className`'s box, cut in its middle when the box is narrower than the
 * name, so its extension stays in sight (FILES2-N1): a card's end-cut showed "q3-report.pdf …"
 * of a name that ended ".exe". The width the name may take is its box's container's (the name
 * fills its label first, Q4-03); a name that fits, a layout that has not measured yet, and a page
 * with nothing laid out (jsdom) all keep the whole name, which the stylesheet still cuts at its
 * end. The whole name is always in the text a caller gives a tooltip or a control. Forwards its
 * ref and props, so a tooltip trigger can be it (`asChild`).
 */
export const MiddleTruncatedName = forwardRef<HTMLSpanElement, MiddleTruncatedNameProps>(
  ({ name, className, ...props }, forwardedRef) => {
    const element = useRef<HTMLSpanElement | null>(null);
    const setElement = useCallback(
      (node: HTMLSpanElement | null) => {
        element.current = node;
        if (typeof forwardedRef === 'function') forwardedRef(node);
        else if (forwardedRef) forwardedRef.current = node;
      },
      [forwardedRef]
    );
    const [shown, setShown] = useState(name);
    useLayoutEffect(() => {
      const own = element.current;
      const box = own?.parentElement;
      if (!own || !box) {
        setShown(name);
        return;
      }
      const fit = () => {
        const width = box.clientWidth;
        // Nothing laid out (jsdom, or not yet): the whole name, and no canvas asked for.
        const context = width > 0 ? measuring() : null;
        if (!context) {
          setShown(name);
          return;
        }
        context.font = window.getComputedStyle(own).font;
        setShown(middleTruncate(name, width, (text) => context.measureText(text).width));
      };
      fit();
      if (typeof ResizeObserver === 'undefined') return;
      const observer = new ResizeObserver(fit);
      observer.observe(box);
      return () => observer.disconnect();
    }, [name]);
    return (
      <span {...props} ref={setElement} className={className} data-crew-file-name="">
        {shown}
      </span>
    );
  }
);
MiddleTruncatedName.displayName = 'MiddleTruncatedName';
