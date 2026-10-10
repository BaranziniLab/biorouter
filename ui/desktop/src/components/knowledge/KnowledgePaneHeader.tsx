import type { ReactNode } from 'react';
import { X } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';

interface Props {
  /** The title element: an `h2` (or a Radix `SheetTitle`) in `text-label`. */
  title: ReactNode;
  /** A mark before the title (a node swatch). */
  leading?: ReactNode;
  /** Controls after the title (the lint drawer's Check). */
  actions?: ReactNode;
  /**
   * The pane's own close, a ghost round button with a Tooltip. Omit it in a `Sheet`, whose
   * primitive draws the close; `.br-knowledge-drawer` aligns that one to this band instead.
   */
  onClose?: () => void;
  closeLabel?: string;
}

/**
 * The Knowledge panes' header: Crew's pane header (`crew/pane/pane.css`, spec 3.12 "Graph and
 * drawers") for the node and edge previews and the change-log and lint drawers.
 *
 * 44px (`--chrome-height`, the band every surface shares), the `--sidebar` ground, a bottom
 * hairline, a `text-label` title whose text starts 16px in (8px of band padding plus the
 * title's own 8px, Crew's `--crew-pane-inset - 8px`), and a ghost round close with a Tooltip.
 * In a drawer the primitive's own close sits at the trailing edge, so the band keeps room for it
 * (`.br-knowledge-pane-header[data-sheet]`), and `.br-knowledge-drawer` centres that close in
 * the band. The geometry is authored in `main.css`.
 */
export function KnowledgePaneHeader({
  title,
  leading,
  actions,
  onClose,
  closeLabel = 'Close',
}: Props) {
  return (
    <div
      className="br-knowledge-pane-header flex h-chrome flex-none items-center border-b border-border-subtle bg-sidebar"
      data-sheet={onClose ? undefined : ''}
    >
      <div className="flex min-w-0 flex-1 items-center gap-2 px-2">
        {leading}
        {title}
      </div>
      {actions}
      {onClose && (
        <Tooltip>
          <TooltipTrigger asChild>
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
          </TooltipTrigger>
          <TooltipContent side="bottom">{closeLabel}</TooltipContent>
        </Tooltip>
      )}
    </div>
  );
}
