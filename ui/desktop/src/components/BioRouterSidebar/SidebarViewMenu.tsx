import { SlidersHorizontal } from '../icons/app-icons';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { sidebarCopy } from './copy';
import {
  SIDEBAR_GROUP_BY_OPTIONS,
  SIDEBAR_SORT_BY_OPTIONS,
  type SidebarChatView,
  type SidebarGroupBy,
  type SidebarSortBy,
} from './sidebarChatView';

interface SidebarViewMenuProps {
  view: SidebarChatView;
  onViewChange: (view: SidebarChatView) => void;
}

/**
 * The Chats header's "View options" menu (owner message 3, Codex parity):
 *
 *   Group by          Date  ›     Date · Folder · None
 *   Sort by  Last activity  ›     Last activity · Created · Name
 *
 * Text only, the current value right-aligned and muted on each row, the choice
 * as a radio item with a check. The trigger carries `aria-haspopup`, so in the
 * overlay sidebar it does not count as a choice and does not close the panel.
 */
export default function SidebarViewMenu({ view, onViewChange }: SidebarViewMenuProps) {
  const { view: copy } = sidebarCopy;
  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              shape="round"
              size="xs"
              className="no-drag text-text-muted hover:text-text-default"
              aria-label={sidebarCopy.chats.viewOptions}
              data-testid="sidebar-view-options"
            >
              <SlidersHorizontal className="size-4" aria-hidden />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>{sidebarCopy.chats.viewOptions}</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="end">
        <DropdownMenuSub>
          <DropdownMenuSubTrigger data-testid="sidebar-view-group-by">
            <span className="flex-1">{copy.groupBy}</span>
            <span className="text-text-muted">{copy.groupByOptions[view.groupBy]}</span>
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent>
            <DropdownMenuRadioGroup
              value={view.groupBy}
              onValueChange={(value) => onViewChange({ ...view, groupBy: value as SidebarGroupBy })}
            >
              {SIDEBAR_GROUP_BY_OPTIONS.map((option) => (
                <DropdownMenuRadioItem key={option} value={option}>
                  {copy.groupByOptions[option]}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger data-testid="sidebar-view-sort-by">
            <span className="flex-1">{copy.sortBy}</span>
            <span className="text-text-muted">{copy.sortByOptions[view.sortBy]}</span>
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent>
            <DropdownMenuRadioGroup
              value={view.sortBy}
              onValueChange={(value) => onViewChange({ ...view, sortBy: value as SidebarSortBy })}
            >
              {SIDEBAR_SORT_BY_OPTIONS.map((option) => (
                <DropdownMenuRadioItem key={option} value={option}>
                  {copy.sortByOptions[option]}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuSubContent>
        </DropdownMenuSub>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
