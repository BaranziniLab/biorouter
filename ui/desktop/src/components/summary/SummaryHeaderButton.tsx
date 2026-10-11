import { useEffect, useId, useRef, useState } from 'react';
import { cn } from '../../utils';
import { PanelRight } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Popover, PopoverAnchor, PopoverContent } from '../ui/popover';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { ChatSummary, type ChatSummaryProps } from '../ChatSummary';
import { summaryCopy } from './copy';

export interface SummaryHeaderButtonProps {
  /**
   * Rung 0's answer for the pane (`useSummaryRail().mode`): `rail` where the
   * docked rail fits, `popover` where it does not.
   */
  mode: 'rail' | 'popover';
  /** The docked rail is shown (rail mode). */
  railShown: boolean;
  /** The rail's element is in the document (shown, or playing its exit). */
  railRendered: boolean;
  /** The id of the rail's `<aside>`, for `aria-controls`. */
  railId: string;
  /**
   * `useSummaryRail().still`: false only in the commit in which the person
   * opened the rail or the chat's first turn brought it in. Anything else that
   * shows or hides the rail is geometry, and the pressed fill then snaps.
   */
  railStill: boolean;
  /** Show or hide the docked rail (rail mode). */
  onToggleRail: () => void;
  popoverOpen: boolean;
  onPopoverOpenChange: (open: boolean) => void;
  /**
   * Set while the rail is folded away and a to-do step is running: the button
   * carries a 6px accent dot and its name gains the count.
   */
  progress: { done: number; total: number } | null;
  summary: ChatSummaryProps;
}

/**
 * The Chat summary's header button: ONE button, two modes (spec 3.5).
 *
 * Where the rail fits it shows and hides the docked rail; where it does not, it
 * opens the same content as a 280px popover. The popover is ANCHORED to the
 * button rather than triggered by it, because a Radix trigger would stamp its
 * own `aria-expanded` and toggle the popover in rail mode too.
 */
export function SummaryHeaderButton({
  mode,
  railShown,
  railRendered,
  railId,
  railStill,
  onToggleRail,
  popoverOpen,
  onPopoverOpenChange,
  progress,
  summary,
}: SummaryHeaderButtonProps) {
  const popoverId = useId();
  const buttonRef = useRef<HTMLButtonElement>(null);
  const railMode = mode === 'rail';
  const expanded = railMode ? railShown : popoverOpen;
  const controls = railMode
    ? railRendered
      ? railId
      : undefined
    : popoverOpen
      ? popoverId
      : undefined;
  const tooltip = railMode
    ? railShown
      ? summaryCopy.hideSummary
      : summaryCopy.showSummary
    : summaryCopy.title;

  // ⚠ Nothing animates during a resize (principle 9, vision V2-R2-6). The
  // pressed fill follows the rail, so dragging the window across the 1344px
  // seam flipped it twice, and the Button's colour transitions played both
  // times while everything else in the chat snapped. A change of the pressed
  // state the person did not make with this button (a resize, the sidebar, a
  // preview, the mode changing at the seam) is drawn at once: `data-still`
  // drops the transitions (summaryRail.css) for that one change and is cleared
  // once a frame has resolved it, which starts no transition because nothing
  // changes then. A press, the popover, and the first turn keep the fade.
  const [pressedByPerson, setPressedByPerson] = useState(false);
  const [seen, setSeen] = useState({ expanded, mode, count: 0 });
  const [stillAt, setStillAt] = useState(0);
  if (seen.expanded !== expanded || seen.mode !== mode) {
    const count = seen.count + 1;
    setSeen({ expanded, mode, count });
    const geometric = seen.mode !== mode || (railMode && railStill && !pressedByPerson);
    if (seen.expanded !== expanded && geometric) setStillAt(count);
    if (pressedByPerson) setPressedByPerson(false);
  }
  // A press that changed nothing (the rail cannot show before a turn) must not
  // vouch for the next change.
  useEffect(() => {
    if (pressedByPerson) setPressedByPerson(false);
  }, [pressedByPerson]);
  useEffect(() => {
    if (stillAt === 0) return;
    let second = 0;
    // Two frames: the first resolves the snapped style, the second clears it.
    const first = requestAnimationFrame(() => {
      second = requestAnimationFrame(() =>
        setStillAt((current) => (current === stillAt ? 0 : current))
      );
    });
    return () => {
      cancelAnimationFrame(first);
      cancelAnimationFrame(second);
    };
  }, [stillAt]);

  return (
    <Popover open={popoverOpen} onOpenChange={onPopoverOpenChange}>
      <PopoverAnchor asChild>
        <span className="inline-flex">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                ref={buttonRef}
                type="button"
                variant="ghost"
                shape="round"
                // Pressed (the rail shown, or its popover open) is the band's
                // fill step, the one every filled control hovers and presses on
                // (R-27); the band is a recessed ground (`--sidebar`).
                className={cn(
                  'no-drag relative text-text-muted hover:text-text-default',
                  expanded && 'bg-background-fill-strong text-text-default'
                )}
                aria-label={
                  progress
                    ? summaryCopy.buttonWithProgress(progress.done, progress.total)
                    : summaryCopy.title
                }
                aria-expanded={expanded}
                aria-controls={controls}
                aria-haspopup={railMode ? undefined : 'dialog'}
                data-summary-mode={mode}
                data-still={stillAt !== 0 ? '' : undefined}
                onClick={() => {
                  setPressedByPerson(true);
                  if (railMode) onToggleRail();
                  else onPopoverOpenChange(!popoverOpen);
                }}
              >
                <PanelRight />
                {progress && <span aria-hidden="true" className="br-summary-dot" />}
              </Button>
            </TooltipTrigger>
            <TooltipContent>{tooltip}</TooltipContent>
          </Tooltip>
        </span>
      </PopoverAnchor>
      <PopoverContent
        id={popoverId}
        role="dialog"
        aria-label={summaryCopy.title}
        side="bottom"
        align="end"
        className="br-summary-popover"
        // The button is outside the content, so a press on it would first
        // dismiss the popover and then reopen it with the click.
        onPointerDownOutside={(event) => {
          if (buttonRef.current?.contains(event.target as Node)) event.preventDefault();
        }}
        // With no Radix trigger, focus is returned to the button by hand.
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          buttonRef.current?.focus({ preventScroll: true });
        }}
      >
        <ChatSummary {...summary} />
      </PopoverContent>
    </Popover>
  );
}
