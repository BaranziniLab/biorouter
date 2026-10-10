import './pickers.css';
import * as React from 'react';
import { Button } from '../ui/button';
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from '../ui/command';
import { InfoTip } from '../ui/info-tip';
import { Switch } from '../ui/switch';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { TOOLS_COPY } from './copy';

/**
 * One picker shell for the Tools popover's three lists (spec 3.7): a search
 * field, a scope line with its InfoTip and the bulk action, then the rows.
 *
 * ⚠ **A combobox over a listbox, not a menu with a text field in it.** Radix's
 * menu typeahead captures printable characters at the menu root, so a search
 * field inside a `DropdownMenu` lost keystrokes to it and focus jumped to the
 * first row whose name started with the letter typed (see `ui/command.tsx`).
 * Here DOM focus stays in the field, the arrow keys move the highlight, Enter
 * toggles the highlighted row and Tab reaches the bulk action.
 *
 * A row is a `role="option"` whose `aria-selected` is its on state, in a
 * multi-select listbox. The switch at its end is the picture of that state, so
 * it is hidden from assistive tech and takes no pointer events: the row is the
 * control.
 */
export interface PickerPanelProps {
  /** The listbox's accessible name ("Extensions"). */
  listLabel: string;
  searchPlaceholder: string;
  query: string;
  onQueryChange: (query: string) => void;
  /** Who a change reaches ("This chat"), with the longer answer in an InfoTip. */
  scopeLabel: string;
  scopeHelp: string;
  /** The ghost bulk action ("Enable all (3)"); omitted when nothing can change. */
  bulk?: { label: string; onClick: () => void; disabled?: boolean } | null;
  /** Shown instead of rows when there are none. */
  emptyText: string;
  /** Focused when the popover opens on this list. */
  inputRef?: React.Ref<HTMLInputElement>;
  children: React.ReactNode;
  'data-testid'?: string;
}

/**
 * The keys `Command` answers at its root for the listbox: the arrows, Home, End and
 * Enter.
 *
 * ⚠ **`Command` handles them wherever the key came from**, and the scope line sits
 * inside its root. So Enter on the scope word's InfoTip, or on the bulk button,
 * bubbled to the root, which toggled whichever row was highlighted and cancelled
 * the Enter: the person asked for "Disable all" or for the explanation and an
 * extension went off instead. (Space was unaffected, which is why a click test
 * missed it.) The scope line's own controls keep these keys.
 */
const LIST_KEYS = new Set(['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter']);

function keepListKeysInTheList(event: React.KeyboardEvent<HTMLElement>) {
  if (LIST_KEYS.has(event.key)) event.stopPropagation();
}

export function PickerPanel({
  listLabel,
  searchPlaceholder,
  query,
  onQueryChange,
  scopeLabel,
  scopeHelp,
  bulk,
  emptyText,
  inputRef,
  children,
  'data-testid': testId,
}: PickerPanelProps) {
  const hasRows = React.Children.toArray(children).length > 0;
  return (
    <Command
      label={listLabel}
      query={query}
      onQueryChange={onQueryChange}
      className="br-picker-panel"
      data-testid={testId}
    >
      <CommandInput
        ref={inputRef}
        placeholder={searchPlaceholder}
        aria-label={searchPlaceholder.replace(/…$/, '')}
      />
      <div className="br-picker-panel__bar" onKeyDown={keepListKeysInTheList}>
        <span className="br-picker-panel__scope">
          <span>{scopeLabel}</span>
          <InfoTip label={TOOLS_COPY.scope.about} help={scopeHelp} />
        </span>
        {bulk && (
          <Button
            type="button"
            variant="ghost"
            size="xs"
            className="px-2 text-secondary text-text-muted"
            onClick={bulk.onClick}
            disabled={bulk.disabled}
          >
            {bulk.label}
          </Button>
        )}
      </div>
      <CommandList aria-label={listLabel} aria-multiselectable className="br-picker-panel__list">
        {hasRows ? (
          children
        ) : (
          <CommandEmpty className="br-picker-panel__empty">{emptyText}</CommandEmpty>
        )}
      </CommandList>
    </Command>
  );
}

export interface PickerRowProps {
  name: string;
  /** The on state: an enabled extension or skill, a visible knowledge base. */
  checked: boolean;
  onToggle: () => void;
  /** Refused or locked: the row stays (a silent omission reads as a broken tool). */
  disabled?: boolean;
  /** A toggle for this row is still in flight. */
  busy?: boolean;
  /** The accessible name, when it must say more than the name ("Soul, visible to this chat"). */
  label?: string;
  /** Beside the name: the `BuiltInBadge`, a bundle's count. */
  badge?: React.ReactNode;
  meta?: string;
  /** A visible second line, for a refusal or a lock: never tooltip-only. */
  note?: string;
  /** What the row is, on hover of its name. Never `title=`, which would rename the row. */
  description?: string;
  'data-testid'?: string;
}

export function PickerRow({
  name,
  checked,
  onToggle,
  disabled = false,
  busy = false,
  label,
  badge,
  meta,
  note,
  description,
  'data-testid': testId,
}: PickerRowProps) {
  const nameNode = <span className="br-picker-row__name">{name}</span>;
  const picture = React.useRef<HTMLButtonElement>(null);
  return (
    <CommandItem
      // The change goes THROUGH the switch picture (below), not around it: the
      // switch slides a change the person made and snaps one that arrived by
      // itself (a list that loaded, another window), and it can only tell the two
      // apart by being clicked. Straight to `onToggle`, every toggle read as
      // "arrived by itself" and the knob jumped.
      onSelect={() => (picture.current ? picture.current.click() : onToggle())}
      selected={checked}
      disabled={disabled}
      aria-label={label}
      // The tooltip is for the pointer; a screen reader hears the same text as
      // the row's description while the arrow keys rest on it. Not a hidden
      // child: text inside the option would become part of its name.
      aria-description={description}
      aria-busy={busy || undefined}
      data-testid={testId}
      className="br-picker-row"
    >
      <span className="br-picker-row__text">
        <span className="br-picker-row__title">
          {description ? (
            <Tooltip>
              <TooltipTrigger asChild>{nameNode}</TooltipTrigger>
              <TooltipContent side="right" className="max-w-[280px]">
                {description}
              </TooltipContent>
            </Tooltip>
          ) : (
            nameNode
          )}
          {badge}
          {meta && <span className="br-picker-row__meta">{meta}</span>}
        </span>
        {note && <span className="br-picker-row__note">{note}</span>}
      </span>
      <span className="br-picker-row__switch" aria-hidden="true">
        <Switch
          ref={picture}
          checked={checked}
          onCheckedChange={() => onToggle()}
          // The click came from the row; it must not bubble back to it.
          onClick={(event) => event.stopPropagation()}
          disabled={disabled}
          tabIndex={-1}
          aria-hidden="true"
        />
      </span>
    </CommandItem>
  );
}
