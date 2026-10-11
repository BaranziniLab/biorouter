import type { ReactNode } from 'react';
import { X } from '../icons/app-icons';
import { Button } from '../ui/button';
import { SheetClose } from '../ui/sheet';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';

interface Props {
  /** The title element: an `h2` (or a Radix `SheetTitle`) in `text-label`. */
  title: ReactNode;
  /** A mark before the title (a node swatch). */
  leading?: ReactNode;
  /** Controls after the title (the lint drawer's Check). */
  actions?: ReactNode;
  /** The pane's own close, for a floating preview that is not a dialog. */
  onClose?: () => void;
  /**
   * A drawer: the close is the Sheet's own `SheetClose`, drawn here so it is the same ghost round
   * button as a preview's. The primitive's corner close is hidden by `.br-knowledge-drawer`.
   */
  sheet?: boolean;
  /** The close's accessible name and Tooltip. */
  closeLabel?: string;
}

/**
 * The Knowledge panes' header: Crew's pane header (`crew/pane/pane.css`, spec 3.12 "Graph and
 * drawers") for the node and edge previews and the change-log and lint drawers.
 *
 * 44px (`--chrome-height`, the band every surface shares), the `--sidebar` ground (a recessed
 * one: a filled control in it, the lint drawer's Check, rests on the strong fill, R-27), a bottom
 * hairline, a `text-label` title whose text starts 16px in (8px of band padding plus the
 * title's own 8px, Crew's `--crew-pane-inset - 8px`), and a ghost round close with a Tooltip,
 * 8px from the edge. The geometry is authored in `main.css`.
 *
 * ⚠ **A drawer draws its own close here, and hides the primitive's.** `ui/sheet.tsx` always
 * renders a corner close (`rounded-element`, 70% ink, no Tooltip, offset 16px so it sits across
 * a 44px band's hairline). Aligning that one from outside depended on it being the content's last
 * child and still left it a different button from the preview's. `SheetClose asChild` over the
 * same `Button` gives the drawer Crew's close exactly, and `.br-knowledge-drawer` hides the
 * primitive's duplicate.
 */
export function KnowledgePaneHeader({
  title,
  leading,
  actions,
  onClose,
  sheet = false,
  closeLabel = 'Close',
}: Props) {
  const close = (
    <Button
      type="button"
      variant="ghost"
      shape="round"
      className="shrink-0"
      aria-label={closeLabel}
      onClick={onClose}
    >
      <X aria-hidden="true" />
    </Button>
  );
  return (
    <div className="br-knowledge-pane-header br-ground-recessed flex h-chrome flex-none items-center border-b border-border-subtle bg-sidebar">
      <div className="flex min-w-0 flex-1 items-center gap-2 px-2">
        {leading}
        {title}
      </div>
      {actions}
      {(onClose || sheet) && (
        <Tooltip>
          <TooltipTrigger asChild>
            {sheet ? <SheetClose asChild>{close}</SheetClose> : close}
          </TooltipTrigger>
          <TooltipContent side="bottom">{closeLabel}</TooltipContent>
        </Tooltip>
      )}
    </div>
  );
}
