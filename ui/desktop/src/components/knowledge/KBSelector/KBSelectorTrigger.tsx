import { useState } from 'react';
import { ChevronDown } from '../../icons/app-icons';
import { Popover, PopoverContent, PopoverTrigger } from '../../ui/popover';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { knowledgeCopy } from '../copy';
import { useKnowledge } from '../KnowledgeContext';
import { KbDot } from '../KbDot';
import { KBSelectorMenu } from './KBSelectorMenu';

interface Props {
  /** Controlled open state. When provided, the trigger acts as a controlled component. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Open the manager dialog (the trigger owns neither the dialog nor its state). */
  onManage: () => void;
  /** Open the manager dialog with the create form unfolded. */
  onCreate: () => void;
  /**
   * How the trigger presents itself.
   *
   * `field` (the default) is the Select-shaped control described below: a
   * full-width filled box, for a form or a rail where it sits among other
   * fields.
   *
   * `subject` is the Knowledge section's subject bar (R-01), where this control
   * IS the page's statement of what it is about. A filled field there would
   * read as one input among several and compete with the `h1` directly above
   * it; the name is set in the band's own label type and the ghost wash appears
   * on hover, so the row reads as an identity that happens to be actionable.
   */
  variant?: 'field' | 'subject';
}

/**
 * The knowledge-base selector — a Select-SHAPED control, so it takes the Select
 * trigger's chrome exactly (ui-spec §4.1).
 *
 * Three corrections over the bespoke field it replaces:
 *
 * - **A filled field, not an outlined one** (R-27, design.md 9.11). `field` rests on the control
 *   ground (`.br-fill-rest`, the subtle fill, or the page ground inside a filled box) and steps
 *   to the strong fill under the pointer (`.br-fill-hover`), with a 1px TRANSPARENT border so
 *   forced colours draw the edge and nothing moves. `subject` stays transparent and takes the
 *   ghost wash (`tint-interactive`) like every other control in the band. The old resting edge
 *   (`--border-emphasized`) and the hover inset ring are gone with the outline.
 * - **It opens an anchored picker, not a 760px modal.** Clicking a selector
 *   should show you the list, not a management console.
 * - **The `KB` badge is dropped**, so the control needs an accessible name of
 *   its own — the surrounding `<h1>` is not one. The name carries the base and
 *   the "+n" count ("Knowledge base: Notes, and 2 more in this chat"), and the
 *   count's explanation is a Tooltip, not a native `title` (spec 3.12).
 *
 * `gap-2` and not `gap-2.5`: 10px is off the 4px grid. `text-label` and not
 * `text-label font-semibold`: 14/600 is not a step in the type scale.
 */
export function KBSelectorTrigger({
  open: openProp,
  onOpenChange,
  onManage,
  onCreate,
  variant = 'field',
}: Props) {
  const { primaryKb, visibleBases } = useKnowledge();
  const [openInternal, setOpenInternal] = useState(false);

  // How many bases this chat uses *besides* the one named in the trigger. With
  // no primary the name slot names nothing, so every visible base is "other" —
  // subtracting 1 unconditionally would undercount them.
  const otherBaseCount = visibleBases.length - (primaryKb ? 1 : 0);

  // Support both controlled (open+onOpenChange) and uncontrolled usage.
  const isControlled = openProp !== undefined;
  const open = isControlled ? openProp : openInternal;
  const setOpen = isControlled ? (v: boolean) => onOpenChange?.(v) : setOpenInternal;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <button
              type="button"
              data-testid="knowledge-kb-selector-trigger"
              aria-label={knowledgeCopy.selector.name(
                primaryKb?.name ?? knowledgeCopy.noPrimary,
                otherBaseCount
              )}
              aria-expanded={open}
              className={
                variant === 'subject'
                  ? // Borderless at rest, content-sized, with the band's own label
                    // type. `min-w-0` so a long base name truncates rather than
                    // pushing the badges and counts out of the row.
                    'biorouter-focus-surface tint-interactive flex h-control-md min-w-0 max-w-[22rem] items-center gap-2 rounded-element border border-transparent px-2 text-label transition-colors'
                  : 'br-fill-rest br-fill-hover biorouter-focus-surface flex h-control-md w-full items-center gap-2 rounded-element border border-transparent px-3 text-label transition-[color,background-color,border-color,box-shadow]'
              }
            >
              <KbDot color={primaryKb?.color} />
              <span className="min-w-0 flex-1 truncate text-left">
                {primaryKb?.name ?? knowledgeCopy.noPrimary}
              </span>
              {otherBaseCount > 0 && (
                <span
                  aria-hidden="true"
                  className="shrink-0 text-supporting tabular-nums text-text-muted"
                >
                  +{otherBaseCount}
                </span>
              )}
              <ChevronDown
                aria-hidden="true"
                className={`h-icon-row w-icon-row shrink-0 text-text-muted transition-transform ${open ? 'rotate-180' : ''}`}
              />
            </button>
          </PopoverTrigger>
        </TooltipTrigger>
        {otherBaseCount > 0 && !open && (
          <TooltipContent side="bottom">
            {knowledgeCopy.selector.others(otherBaseCount)}
          </TooltipContent>
        )}
      </Tooltip>
      <PopoverContent
        align="start"
        sideOffset={6}
        className="w-64 p-0"
        // The picker owns a text field, so the popover must not steal its focus
        // back to the trigger on open.
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <KBSelectorMenu
          onClose={() => setOpen(false)}
          onManage={() => {
            setOpen(false);
            onManage();
          }}
          onCreate={() => {
            setOpen(false);
            onCreate();
          }}
        />
      </PopoverContent>
    </Popover>
  );
}
