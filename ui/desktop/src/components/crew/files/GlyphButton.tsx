import { forwardRef, type ComponentProps, type ReactNode } from 'react';
import { Button } from '../../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { MoreHorizontal } from '../../icons/app-icons';
import { DropdownMenuTrigger } from '../../ui/dropdown-menu';
import { filesCopy } from './copy';
import './files.css';

/**
 * A glyph-only control inside a 24px chip (remove, pause, resume): a 16px target whose
 * accessible name is also its tooltip, per the spec's rule for glyph-only buttons. Focus is the
 * app's focused-control fill (D-15), not a ring: `.crew-chip-action:focus-visible` in `files.css`
 * restates it, because that file's unlayered muted ink would otherwise beat the base layer's.
 */
export function ChipAction({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button type="button" className="crew-chip-action" aria-label={label} onClick={onClick}>
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

/**
 * The `⋯` that opens a file row's menu: a ghost round 28px button named for the row ("More
 * actions for counts.csv"), with a tooltip in the same words. Render it inside a
 * `DropdownMenu`; it is that menu's trigger.
 */
export const MoreActionsTrigger = forwardRef<
  HTMLButtonElement,
  { name: string } & Omit<ComponentProps<typeof Button>, 'children' | 'name'>
>(function MoreActionsTrigger({ name, ...props }, ref) {
  const label = filesCopy.fileActions(name);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <DropdownMenuTrigger asChild>
          <Button
            ref={ref}
            type="button"
            variant="ghost"
            size="sm"
            shape="round"
            aria-label={label}
            {...props}
          >
            <MoreHorizontal aria-hidden />
          </Button>
        </DropdownMenuTrigger>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
});
