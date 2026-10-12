import { useId, useMemo, useState } from 'react';
import { Check, ChevronDown, Search } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Checkbox } from '../../ui/Checkbox';
import { InfoTip } from '../../ui/info-tip';
import { Input } from '../../ui/input';
import { Note } from '../../ui/note';
import { Popover, PopoverContent, PopoverTrigger } from '../../ui/popover';
import { cn } from '../../../utils';
import { workflowFormCopy } from '../copy';

const copy = workflowFormCopy.picker;

export interface WorkflowResourceItem {
  id: string;
  label: string;
  description?: string | null;
  badge?: string;
}

interface WorkflowResourcePickerProps {
  label: string;
  /** What the picker is for: help behind an InfoTip beside the label, never a paragraph. */
  description?: string;
  items: WorkflowResourceItem[];
  selectedIds: string[];
  onSelectedIdsChange: (ids: string[]) => void;
  defaultId?: string | null;
  onDefaultIdChange?: (id: string | null) => void;
  /**
   * What "no default" MEANS here, behind the InfoTip on the card's "No default" line, which
   * shows whenever a default could be named and none is.
   *
   * The rows, and with them the only mark of which item is the default, live inside a closed
   * popover, so a card with a default and a card with none would read identically from outside
   * it. That is fine for a control whose unset state is obvious and wrong for one whose purpose
   * is a default: the reader cannot tell a captured "this chat has no primary base" from a bug.
   * So the card says "No default" in words, and the consequence is one hover or Tab away. The
   * wording belongs to the caller because what the absence costs is domain-specific.
   */
  noDefaultText?: string;
  /** A standing condition the selection is subject to, shown under the label. */
  notice?: string;
  emptyText: string;
  searchPlaceholder: string;
  noun: string;
}

/**
 * Pick items from a set (knowledge bases, skills, extensions) for a workflow.
 *
 * Set membership is a `Checkbox` on the left of its label (spec 2.6, principle 5; Crew's
 * PersonPicker), never a Switch: a switch applies a setting now, and these choices are only
 * submitted with the form. Each checkbox is named by its item's label, and its description is
 * the box's description.
 */
export function WorkflowResourcePicker({
  label,
  description,
  items,
  selectedIds,
  onSelectedIdsChange,
  defaultId,
  onDefaultIdChange,
  noDefaultText,
  notice,
  emptyText,
  searchPlaceholder,
  noun,
}: WorkflowResourcePickerProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const selected = useMemo(() => new Set(selectedIds), [selectedIds]);
  const idBase = useId().replace(/[^a-zA-Z0-9_-]/g, '');
  const labelId = `${idBase}-label`;

  // Named by id, labelled by label: the id is what is saved, and a picker whose
  // items have not loaded yet must still be able to say a default is set.
  const defaultLabel = defaultId
    ? (items.find((item) => item.id === defaultId)?.label ?? defaultId)
    : null;
  // Nothing selected already says its own thing on the trigger ("No KBs
  // selected"), and "no default" on top of it is noise about a set that is empty.
  const showDefault = Boolean(onDefaultIdChange) && selectedIds.length > 0;
  const showNoDefault = showDefault && !defaultLabel && Boolean(noDefaultText);

  const filteredItems = useMemo(() => {
    const q = query.trim().toLowerCase();
    const filtered = q
      ? items.filter((item) =>
          [item.label, item.id, item.description, item.badge]
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

  // Checking an item on or off changes the selection, never the default: only
  // the Default control names one. For knowledge bases the default becomes the
  // primary of every chat the workflow starts, which is where KB-less writes
  // go, and the daemon never infers that pointer (`plan_knowledge_selection`).
  // So a base checked on is not made the default, and unchecking the default
  // leaves none rather than passing the role to the first base left.
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
    <div className="flex flex-col gap-1.5" role="group" aria-labelledby={labelId}>
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 flex-col">
          <div className="flex min-w-0 items-center">
            <span id={labelId} className="text-label text-text-default">
              {label}
            </span>
            {description && <InfoTip label={label} help={description} />}
          </div>
          {showDefault && (defaultLabel || showNoDefault) && (
            <p
              className="flex min-w-0 items-center text-supporting text-text-muted"
              data-testid="resource-picker-default-summary"
            >
              <span className="truncate">
                {defaultLabel ? copy.defaultIs(defaultLabel) : copy.noDefault}
              </span>
              {!defaultLabel && noDefaultText && (
                <InfoTip label={copy.noDefault} help={noDefaultText} />
              )}
            </p>
          )}
        </div>
        <Popover open={open} onOpenChange={setOpen}>
          <PopoverTrigger asChild>
            <Button type="button" variant="secondary" className="shrink-0">
              {copy.count(selectedIds.length, noun)}
              <ChevronDown aria-hidden />
            </Button>
          </PopoverTrigger>
          <PopoverContent
            align="end"
            className="w-[min(440px,calc(100vw-32px))] p-1"
            onOpenAutoFocus={(event) => event.preventDefault()}
          >
            <div className="relative p-1">
              <Search
                aria-hidden
                className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-text-muted"
              />
              <Input
                type="search"
                aria-label={searchPlaceholder}
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={searchPlaceholder}
                className="pl-8"
                autoFocus
              />
            </div>
            <div className="max-h-72 overflow-y-auto pt-1">
              {filteredItems.length === 0 ? (
                <div className="px-3 py-6 text-center text-supporting text-text-muted">
                  {emptyText}
                </div>
              ) : (
                filteredItems.map((item, index) => {
                  const isSelected = selected.has(item.id);
                  const isDefault = defaultId === item.id;
                  const checkboxId = `${idBase}-item-${index}`;
                  const nameId = `${checkboxId}-name`;
                  const descriptionId = item.description ? `${checkboxId}-description` : undefined;
                  return (
                    <div
                      key={item.id}
                      className="flex min-h-8 items-center gap-2 rounded-element px-2 py-1 hover:bg-overlay-hover"
                    >
                      {/* The whole label toggles the box; the Default control is its
                          sibling, so pressing it never checks or unchecks the item. */}
                      <label
                        htmlFor={checkboxId}
                        className="flex min-w-0 flex-1 cursor-pointer items-center gap-2"
                      >
                        <Checkbox
                          id={checkboxId}
                          checked={isSelected}
                          onChange={() => toggleSelected(item.id)}
                          aria-labelledby={nameId}
                          aria-describedby={descriptionId}
                        />
                        <span className="min-w-0 flex-1">
                          <span className="flex min-w-0 items-center gap-2">
                            <span id={nameId} className="truncate text-label text-text-default">
                              {item.label}
                            </span>
                            {item.badge && <Badge tone="neutral">{item.badge}</Badge>}
                          </span>
                          {item.description && (
                            <span
                              id={descriptionId}
                              className="block truncate text-supporting text-text-muted"
                            >
                              {item.description}
                            </span>
                          )}
                        </span>
                      </label>
                      {onDefaultIdChange && isSelected && (
                        // A toggle, so "selected, and no default" stays reachable
                        // once a default has been named.
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          aria-pressed={isDefault}
                          aria-label={copy.defaultNamed(noun, item.label)}
                          className={cn(isDefault ? 'text-text-default' : 'text-text-muted')}
                          onClick={() => onDefaultIdChange(isDefault ? null : item.id)}
                        >
                          {isDefault && <Check aria-hidden />}
                          {copy.default}
                        </Button>
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
