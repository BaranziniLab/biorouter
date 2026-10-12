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
 *
 * The picture is still named, by the row's visible name through
 * `aria-labelledby`, as every switch on the Extensions page is (UI defect 3,
 * final check 2026-10-12: anything that reads the DOM found three lists of
 * unnamed switches). Its wrapper hides it, not an `aria-hidden` of its own: an
 * element hidden by its own attribute has no name at all.
 *
 * ⚠ **So the option is named by reference, never from its content.** Chromium
 * reads a hidden descendant into a name built from content in more than one
 * case: a focusable element under `aria-hidden` (the switch is a `<button>`,
 * focusable at `tabIndex={-1}`), and any element with generated content (the
 * switch's `::after` hit target). Either one made every option say its name
 * twice once the switch had one ("extensions 1 extensions 1"), measured in
 * Chromium's own accessibility tree, where jsdom and Playwright's aria snapshot
 * both said it once; `inert` on the wrapper did not help. So the option points at
 * its visible text (the title line, then the note) with `aria-labelledby`, or
 * carries `label` as its `aria-label`, and nothing else inside it can reach its
 * name. `toolsPopover.browser.test.tsx` reads Chromium's tree to pin it.
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
      <div className="br-picker-panel__bar">
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
  /** What the row is, on hover of its name line. Never `title=`, which would rename the row. */
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
  // The switch's name: the row's name alone. Not `label`, which may say the
  // state ("Soul, visible to this chat"), and a switch's name never changes with
  // its state (spec 2.6); not the badge or the count beside it either.
  const nameId = React.useId();
  // The option's name, by reference (see the file's header): the title line,
  // then the note. `label` replaces both, so the two are never set together
  // (`aria-labelledby` would win over `aria-label`).
  const titleId = React.useId();
  const noteId = React.useId();
  const labelledBy = label ? undefined : note ? `${titleId} ${noteId}` : titleId;
  // The name, its badge and its count are one line, and the line is what the
  // description hangs from: a pointer resting on the "Built-in" badge gets the
  // row's tooltip too (the badge cannot hold a help glyph of its own inside a
  // row that is itself the control).
  const title = (
    <span id={titleId} className="br-picker-row__title">
      <span id={nameId} className="br-picker-row__name">
        {name}
      </span>
      {badge}
      {meta && <span className="br-picker-row__meta">{meta}</span>}
    </span>
  );
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
      aria-labelledby={labelledBy}
      // The tooltip is for the pointer; a screen reader hears the same text as
      // the row's description while the arrow keys rest on it. Not a hidden
      // child: text inside the option would become part of its name.
      aria-description={description}
      aria-busy={busy || undefined}
      data-testid={testId}
      className="br-picker-row"
    >
      <span className="br-picker-row__text">
        {description ? (
          <Tooltip>
            <TooltipTrigger asChild>{title}</TooltipTrigger>
            <TooltipContent side="right" className="max-w-[280px]">
              {description}
            </TooltipContent>
          </Tooltip>
        ) : (
          title
        )}
        {note && (
          <span id={noteId} className="br-picker-row__note">
            {note}
          </span>
        )}
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
          // Hidden by the wrapper, never by its own `aria-hidden` (see the file's
          // header): named like the Extensions page's switches.
          aria-labelledby={nameId}
        />
      </span>
    </CommandItem>
  );
}
