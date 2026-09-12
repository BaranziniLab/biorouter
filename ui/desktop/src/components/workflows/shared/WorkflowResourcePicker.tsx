import { useMemo, useState } from 'react';
import { Check, ChevronDown, Search } from '../../icons/app-icons';
import { Note } from '../../ui/note';
import { Popover, PopoverContent, PopoverTrigger } from '../../ui/popover';
import { Switch } from '../../ui/switch';
import { cn } from '../../../utils';

export interface WorkflowResourceItem {
  id: string;
  label: string;
  description?: string | null;
  badge?: string;
}

interface WorkflowResourcePickerProps {
  label: string;
  description?: string;
  items: WorkflowResourceItem[];
  selectedIds: string[];
  onSelectedIdsChange: (ids: string[]) => void;
  defaultId?: string | null;
  onDefaultIdChange?: (id: string | null) => void;
  /** A standing condition the selection is subject to, shown under the label. */
  notice?: string;
  emptyText: string;
  searchPlaceholder: string;
  noun: string;
}

function countLabel(count: number, noun: string): string {
  if (count === 0) return `No ${noun}s selected`;
  if (count === 1) return `1 ${noun} selected`;
  return `${count} ${noun}s selected`;
}

export function WorkflowResourcePicker({
  label,
  description,
  items,
  selectedIds,
  onSelectedIdsChange,
  defaultId,
  onDefaultIdChange,
  notice,
  emptyText,
  searchPlaceholder,
  noun,
}: WorkflowResourcePickerProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const selected = useMemo(() => new Set(selectedIds), [selectedIds]);

  const filteredItems = useMemo(() => {
    const q = query.trim().toLowerCase();
    const filtered = q
      ? items.filter((item) =>
          [item.label, item.description, item.badge]
            .filter(Boolean)
            .some((value) => value!.toLowerCase().includes(q))
        )
      : items;

    return [...filtered].sort((a, b) => {
      const selectedDelta = Number(selected.has(b.id)) - Number(selected.has(a.id));
      if (selectedDelta !== 0) return selectedDelta;
      return a.label.localeCompare(b.label);
    });
  }, [items, query, selected]);

  // Switching an item on or off changes the selection, never the default: only
  // the Default control names one. For knowledge bases the default becomes the
  // primary of every chat the workflow starts, which is where KB-less writes
  // go, and the daemon never infers that pointer (`plan_knowledge_selection`).
  // So a base switched on is not made the default, and switching the default
  // off leaves none rather than passing the role to the first base left.
  const toggleSelected = (id: string) => {
    if (selected.has(id)) {
      onSelectedIdsChange(selectedIds.filter((selectedId) => selectedId !== id));
      if (defaultId === id) {
        onDefaultIdChange?.(null);
      }
      return;
    }

    onSelectedIdsChange([...selectedIds, id]);
  };

  return (
    <div className="space-y-2">
      <div className="flex items-start justify-between gap-3">
        <div>
          <label className="text-label text-text-default">{label}</label>
          {description && <p className="mt-0.5 text-supporting text-text-muted">{description}</p>}
        </div>
        <Popover open={open} onOpenChange={setOpen}>
          <PopoverTrigger asChild>
            <button
              type="button"
              className="inline-flex h-8 shrink-0 items-center gap-2 rounded-element bg-background-medium px-2.5 text-supporting text-text-default transition-colors hover:bg-overlay-hover "
            >
              <span>{countLabel(selectedIds.length, noun)}</span>
              <ChevronDown
                className={cn('h-3.5 w-3.5 transition-transform', open && 'rotate-180')}
              />
            </button>
          </PopoverTrigger>
          <PopoverContent
            align="end"
            className="w-[min(440px,calc(100vw-32px))] border-border-subtle p-2"
            onOpenAutoFocus={(event) => event.preventDefault()}
          >
            <div className="flex h-8 items-center gap-2 rounded-element bg-background-default px-2 ring-1 ring-border-input">
              <Search className="h-3.5 w-3.5 text-text-muted" />
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={searchPlaceholder}
                className="h-full min-w-0 flex-1 bg-transparent text-body text-text-default placeholder:text-text-muted"
                autoFocus
              />
            </div>
            <div className="mt-2 max-h-72 overflow-y-auto">
              {filteredItems.length === 0 ? (
                <div className="px-2 py-6 text-center text-supporting text-text-muted">
                  {emptyText}
                </div>
              ) : (
                filteredItems.map((item) => {
                  const isSelected = selected.has(item.id);
                  const isDefault = defaultId === item.id;
                  return (
                    <div
                      key={item.id}
                      className={cn(
                        'flex cursor-pointer items-center gap-3 rounded-element px-2 py-2 transition-colors',
                        isSelected ? 'bg-overlay-selected' : 'hover:bg-overlay-hover'
                      )}
                      onClick={() => toggleSelected(item.id)}
                    >
                      <Switch
                        checked={isSelected}
                        onCheckedChange={() => toggleSelected(item.id)}
                        onClick={(event) => event.stopPropagation()}
                        variant="mono"
                        aria-label={`Toggle ${item.label}`}
                      />
                      <div className="min-w-0 flex-1">
                        <div className="flex min-w-0 items-center gap-2">
                          <span className="truncate text-label text-text-default">
                            {item.label}
                          </span>
                          {item.badge && (
                            <span className="shrink-0 rounded-inner bg-background-muted px-1.5 py-0.5 text-caps text-text-muted">
                              {item.badge}
                            </span>
                          )}
                        </div>
                        {item.description && (
                          <p className="truncate text-supporting text-text-muted">
                            {item.description}
                          </p>
                        )}
                      </div>
                      {onDefaultIdChange && isSelected && (
                        // A toggle, so "selected, and no default" stays reachable
                        // once a default has been named.
                        <button
                          type="button"
                          aria-pressed={isDefault}
                          aria-label={`Default ${noun}: ${item.label}`}
                          className={cn(
                            'inline-flex h-6 shrink-0 items-center gap-1 rounded-element px-1.5 text-supporting transition-colors',
                            isDefault
                              ? 'bg-background-default text-text-default'
                              : 'text-text-muted hover:bg-overlay-hover hover:text-text-default'
                          )}
                          onClick={(event) => {
                            event.stopPropagation();
                            onDefaultIdChange(isDefault ? null : item.id);
                          }}
                        >
                          {isDefault && <Check className="h-3 w-3" />}
                          Default
                        </button>
                      )}
                    </div>
                  );
                })
              )}
            </div>
          </PopoverContent>
        </Popover>
      </div>
      {notice && <Note role="status">{notice}</Note>}
    </div>
  );
}
