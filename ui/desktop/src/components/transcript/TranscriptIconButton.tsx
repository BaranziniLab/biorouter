import * as React from 'react';
import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { cn } from '../../utils';

interface TranscriptIconButtonProps extends Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  'children'
> {
  /** The accessible name. It follows the state ("Copy", then "Copied"). */
  label: string;
  /** What the tooltip says, when it differs from the name. */
  tip?: React.ReactNode;
  /**
   * Hold the tooltip open: the button answers a press in its tooltip ("Copied")
   * rather than by changing its own width (Crew's `CopyIconButton`).
   */
  holdTip?: boolean;
  /** A 14px glyph. */
  icon: React.ReactNode;
  /** Warning ink, for an outcome that failed. */
  tone?: 'default' | 'warning';
}

/**
 * The transcript's one icon action: a 24px ghost square with a 14px glyph and a
 * tooltip that names it. Message actions, a code block's Copy and Run, and the
 * ⋯ trigger all use it, so the transcript has one action look.
 */
export const TranscriptIconButton = React.forwardRef<HTMLButtonElement, TranscriptIconButtonProps>(
  function TranscriptIconButton(
    { label, tip, holdTip = false, icon, tone = 'default', className, ...props },
    ref
  ) {
    const [hovered, setHovered] = React.useState(false);
    return (
      <Tooltip open={holdTip || hovered} onOpenChange={setHovered}>
        <TooltipTrigger asChild>
          <Button
            ref={ref}
            type="button"
            variant="ghost"
            size="xs"
            shape="round"
            aria-label={label}
            className={cn(
              '[&_svg]:size-3.5',
              tone === 'warning'
                ? 'text-text-warning hover:text-text-warning'
                : 'text-text-muted hover:text-text-default',
              className
            )}
            {...props}
          >
            {icon}
          </Button>
        </TooltipTrigger>
        <TooltipContent>{tip ?? label}</TooltipContent>
      </Tooltip>
    );
  }
);
