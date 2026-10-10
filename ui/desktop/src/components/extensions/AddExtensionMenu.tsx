import { Button } from '../ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '../ui/dropdown-menu';
import { RowActionMenuItems, type RowActionItem } from '../ui/row-actions';
import { Plus } from '../icons/app-icons';
import { EXTENSIONS_COPY } from './copy';

/**
 * The band's one primary action: a menu of the three ways an extension arrives. Text-only, like
 * every view menu (spec 2.6), and the same shape as the Skills band's Add menu.
 *
 * It replaces three labelled buttons, of which the PRIMARY one opened the least common path
 * (installing from a file) and the other two were `outline`, not the secondary the rest of the
 * app uses.
 */
export function AddExtensionMenu({
  onBrowse,
  onFromFile,
  onCustom,
}: {
  onBrowse: () => void;
  onFromFile: () => void;
  onCustom: () => void;
}) {
  const items: RowActionItem[] = [
    { label: EXTENSIONS_COPY.browse, onSelect: onBrowse },
    { label: EXTENSIONS_COPY.fromFile, onSelect: onFromFile },
    { label: EXTENSIONS_COPY.custom, onSelect: onCustom },
  ];
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button>
          <Plus aria-hidden />
          {EXTENSIONS_COPY.add}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <RowActionMenuItems items={items} />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
