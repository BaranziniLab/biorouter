import * as React from 'react';

import { cn } from '../../utils';
import { Tooltip, TooltipContent, TooltipTrigger } from './Tooltip';

/**
 * Whether a one-line element is cut off: its text is wider than its box. Measured after layout and
 * again whenever the box resizes, so a line that fits in a wide window and is cut in a narrow one
 * gains and loses its tooltip with the window. jsdom lays nothing out, so it is never cut there.
 */
export function useIsTruncated(ref: React.RefObject<HTMLElement | null>, text: string): boolean {
  const [truncated, setTruncated] = React.useState(false);
  React.useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => setTruncated(element.scrollWidth > element.clientWidth + 1);
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref, text]);
  return truncated;
}

export interface TruncatedLineProps {
  /** The whole line. It is drawn on one line, and the tooltip holds it while it is cut off. */
  text: string;
  /** The element the line is drawn as. */
  as?: 'span' | 'p';
  /** Type and ink only (`text-supporting text-text-muted`); the line truncates itself. */
  className?: string;
  /**
   * Whether a cut-off line takes a Tab stop, so a keyboard user can open the tooltip that holds
   * the rest. Off inside a `<label>` row, where a focusable line would be a control inside the
   * control the row already is.
   */
  focusable?: boolean;
  'data-testid'?: string;
}

/**
 * A one-line row description whose cut-off end is a hover away (owner, 2026-11-13: no text that
 * cannot be read, and the detail behind a hover rather than on the page).
 *
 * A list row shows its description on one line with an ellipsis. Until this, the clipped part of
 * a marketplace, workflow or built app description could not be read anywhere. The tooltip opens
 * only while the line is really cut off: over a line that fits it would only say the same thing
 * again, over the next row (the provider catalog's rule, `ProviderCard`). A screen reader already
 * hears the whole line, since the ellipsis is drawn, not written.
 */
export function TruncatedLine({
  text,
  as = 'span',
  className,
  focusable = true,
  'data-testid': testId,
}: TruncatedLineProps) {
  const ref = React.useRef<HTMLElement | null>(null);
  const truncated = useIsTruncated(ref, text);
  const [open, setOpen] = React.useState(false);
  const Comp = as;

  return (
    <Tooltip open={truncated && open} onOpenChange={setOpen}>
      <TooltipTrigger asChild>
        <Comp
          ref={ref as React.Ref<HTMLParagraphElement & HTMLSpanElement>}
          tabIndex={truncated && focusable ? 0 : undefined}
          data-truncated={truncated ? 'true' : undefined}
          data-testid={testId}
          className={cn('block min-w-0 truncate', className)}
        >
          {text}
        </Comp>
      </TooltipTrigger>
      <TooltipContent side="bottom" align="start">
        {text}
      </TooltipContent>
    </Tooltip>
  );
}
