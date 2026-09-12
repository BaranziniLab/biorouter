import { useMemo, useState } from 'react';
import { BookOpen } from '../icons/app-icons';
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { Input } from '../ui/input';
import { Switch } from '../ui/switch';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import BuiltInBadge from '../ui/BuiltInBadge';
import { BUILTIN_RECREATED_TITLE, isBuiltinKnowledgeBase } from '../../utils/builtins';
import { useKnowledge } from '../knowledge/KnowledgeContext';

export function BottomMenuKnowledgeSelection() {
  const {
    bases,
    visibleBases,
    hiddenKbIds,
    setHiddenKbIds,
    toggleKbHidden,
    hideAllKnowledgeBases,
    showAllKnowledgeBases,
    refresh,
  } = useKnowledge();
  const [open, setOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');

  const filteredBases = useMemo(
    () =>
      bases.filter((base) => base.name.toLowerCase().includes(searchQuery.trim().toLowerCase())),
    [bases, searchQuery]
  );

  const filteredVisibleCount = useMemo(
    () => filteredBases.filter((base) => !hiddenKbIds.includes(base.id)).length,
    [filteredBases, hiddenKbIds]
  );

  const handleBulkToggle = () => {
    if (filteredBases.length === 0) return;

    const targetVisible = filteredVisibleCount === 0;
    if (!searchQuery.trim()) {
      if (targetVisible) {
        showAllKnowledgeBases();
      } else {
        hideAllKnowledgeBases();
      }
      return;
    }

    // One complete set, one write. Toggling id by id looks equivalent but is
    // not: every `toggleKbHidden` derives its next set from the same captured
    // `hiddenKbIds`, so each call overwrites the one before it and only the last
    // base in the filter actually changes — and each is its own daemon
    // round-trip. Bases the search did not match keep whatever they had.
    const filteredIds = new Set(filteredBases.map((base) => base.id));
    const nextHidden = targetVisible
      ? hiddenKbIds.filter((id) => !filteredIds.has(id))
      : [...hiddenKbIds, ...filteredIds];
    setHiddenKbIds(nextHidden);
  };

  return (
    <DropdownMenu
      open={open}
      onOpenChange={(nextOpen) => {
        setOpen(nextOpen);
        if (!nextOpen) setSearchQuery('');
        // Re-read on open, as the Knowledge view's picker and manager do: the
        // provider already follows a turn's end, but a base created from the
        // CLI or another window changes nothing this renderer can hear.
        if (nextOpen) void refresh();
      }}
    >
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              // `text-supporting`, not the `text-secondary` main.css prescribes
              // for a dense control — the override is explained once in
              // ChatInput.tsx, search "THE RAILS' TYPE".
              className="flex h-7 items-center rounded-md px-0.5 cursor-pointer [&_svg]:size-4 text-text-default/70 hover:bg-background-medium hover:text-text-default text-supporting"
              aria-label={`Manage knowledge bases (${visibleBases.length} visible)`}
            >
              <BookOpen className="mr-0.5 h-4 w-4" />
              <span>{visibleBases.length}</span>
            </button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent side="top">Manage knowledge bases</TooltipContent>
      </Tooltip>
      <DropdownMenuContent
        side="top"
        align="center"
        className="w-64"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        <div className="p-2">
          <Input
            type="text"
            placeholder="Search knowledge bases..."
            value={searchQuery}
            onChange={(event) => setSearchQuery(event.target.value)}
            className="h-8"
            autoFocus
          />
          {filteredBases.length > 0 && (
            <button
              type="button"
              onClick={handleBulkToggle}
              className="mt-1.5 cursor-pointer text-supporting text-text-default/70 underline-offset-2 hover:text-text-default hover:underline"
            >
              {filteredVisibleCount === 0
                ? `Show all (${filteredBases.length})`
                : `Hide all (${filteredVisibleCount})`}
            </button>
          )}
        </div>
        <div className="max-h-[400px] overflow-y-auto">
          {filteredBases.length === 0 ? (
            <div className="px-3 py-4 text-center text-secondary text-text-muted">
              {searchQuery ? 'No knowledge bases found' : 'No knowledge bases available'}
            </div>
          ) : (
            filteredBases.map((base) => {
              const hidden = hiddenKbIds.includes(base.id);
              return (
                <DropdownMenuCheckboxItem
                  key={base.id}
                  checked={!hidden}
                  showIndicator={false}
                  onCheckedChange={() => toggleKbHidden(base.id)}
                  onSelect={(event) => event.preventDefault()}
                  className="flex cursor-pointer items-center justify-between px-2 py-2 transition-colors duration-[var(--motion-fast)] hover:bg-background-medium"
                  // One verb pair per popup (#82 category 5). The trigger's
                  // label, the bulk button and this row all have to say
                  // VISIBILITY, or the switch reads as the enable/disable
                  // toggle the other two popups use for a different thing.
                  aria-label={`${base.name}, ${hidden ? 'hidden from' : 'visible to'} this chat`}
                >
                  <div className="flex min-w-0 items-center gap-1.5 pr-2">
                    {/* Siblings pass a `title` so a truncated name is still
                        readable; this row had none. */}
                    <div className="truncate font-medium text-text-default" title={base.name}>
                      {base.name}
                    </div>
                    {isBuiltinKnowledgeBase(base.id) && (
                      <BuiltInBadge title={BUILTIN_RECREATED_TITLE} />
                    )}
                  </div>
                  <div className="pointer-events-none" aria-hidden="true">
                    <Switch checked={!hidden} variant="mono" tabIndex={-1} aria-hidden="true" />
                  </div>
                </DropdownMenuCheckboxItem>
              );
            })
          )}
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
