import './pickers.css';
import * as React from 'react';
import { ChevronRight } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../ui/collapsible';
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
 * name. `composer/plusPopover.browser.test.tsx` reads Chromium's tree to pin it.
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
  /**
   * Shown instead of rows when there are none. `null` shows no empty line: a
   * search that matched only rows in `trailing` has nothing to apologise for.
   */
  emptyText: string | null;
  /**
   * More to say about an empty list, in an InfoTip beside `emptyText` (design.md
   * 9.16: one line, then helper text). `about` names it ("About built-in
   * capabilities"), `text` is the help, two sentences at most.
   */
  emptyHelp?: { about: string; text: string };
  /** Focused when the popover opens on this list. */
  inputRef?: React.Ref<HTMLInputElement>;
  /**
   * A section after the rows, or after the empty line, in the same scrolling
   * list: the built-in capabilities or contexts (`PickerDisclosure`). It is not
   * a row, so a list with none of the person's own still says why above it.
   */
  trailing?: React.ReactNode;
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
  emptyHelp,
  inputRef,
  trailing,
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
        ) : emptyText === null ? null : (
          <CommandEmpty className="br-picker-panel__empty">
            {emptyHelp ? (
              // One centred group: the line, then its glyph 4px after the last word. The
              // InfoTip keeps its own keys: `Command` leaves any target but the field alone.
              <span className="br-picker-panel__empty-line">
                <span>{emptyText}</span>
                <InfoTip label={emptyHelp.about} help={emptyHelp.text} />
              </span>
            ) : (
              emptyText
            )}
          </CommandEmpty>
        )}
        {trailing}
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

export interface PickerDisclosureProps {
  /** The trigger's words and the group's name ("Built-in capabilities"). */
  label: string;
  /** Beside the label in muted ink, and the trigger's description ("12 on"). */
  summary?: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** `PickerRow`s, mounted only while open. */
  children: React.ReactNode;
  'data-testid'?: string;
}

/**
 * The built-in rows at the end of a list, behind one row that opens them
 * (owner, 2026-11-13: "a little Advanced button … the option to expand"). The
 * person's own extensions or skills come first and stay the list; what ships
 * with Biorouter waits one click away (simplicity first, complexity hidden).
 *
 * A group in the listbox, named by its trigger. The trigger is a real button in
 * the row recipe, so Tab reaches it and Enter or Space opens it: `Command`
 * leaves the keys of any target but its field alone. Its rows are ordinary
 * options, so once open the arrow keys walk them like the rows above.
 *
 * ⚠ **The trigger is not an option.** An option is a row a person switches,
 * and every count of options (the tests', a screen reader's "1 of 3") would
 * then include a row that is not one. The content is Radix `Collapsible`, which
 * unmounts it while closed, so a closed group adds no option either; it opens
 * with the shared disclosure animation (`.biorouter-disclosure-panel`).
 */
export function PickerDisclosure({
  label,
  summary,
  open,
  onOpenChange,
  children,
  'data-testid': testId,
}: PickerDisclosureProps) {
  const labelId = React.useId();
  const summaryId = React.useId();
  return (
    <Collapsible
      open={open}
      onOpenChange={onOpenChange}
      role="group"
      aria-labelledby={labelId}
      className="br-picker-disclosure"
      data-testid={testId}
    >
      <CollapsibleTrigger asChild>
        <button
          type="button"
          className="br-picker-disclosure__trigger"
          aria-labelledby={labelId}
          aria-describedby={summary ? summaryId : undefined}
        >
          <span id={labelId} className="br-picker-disclosure__label">
            {label}
          </span>
          {summary ? (
            <>
              <span aria-hidden="true" className="br-picker-disclosure__summary">
                ·
              </span>
              <span id={summaryId} className="br-picker-disclosure__summary">
                {summary}
              </span>
            </>
          ) : null}
          <ChevronRight aria-hidden="true" className="br-picker-disclosure__chevron" />
        </button>
      </CollapsibleTrigger>
      <CollapsibleContent className="biorouter-disclosure-panel">
        <div className="br-picker-disclosure__rows">{children}</div>
      </CollapsibleContent>
    </Collapsible>
  );
}

/**
 * Whether a list's built-in section is open: collapsed whenever the list comes
 * on screen, and opened by a search that matches a built-in row, so a person
 * who types "Developer" sees it rather than "No extensions found". After that
 * the person's own click decides.
 *
 * @param listOpen the list is on screen; closing it collapses the section.
 * @param matching a non-empty search matches at least one built-in row.
 */
export function useBuiltInDisclosure(
  listOpen: boolean,
  matching: boolean
): [boolean, (open: boolean) => void] {
  const [expanded, setExpanded] = React.useState(false);
  // Adjusted during render rather than in an effect, so a search that opens the
  // section does not paint one frame of it closed (react.dev, "Adjusting some
  // state when a prop changes").
  const [seen, setSeen] = React.useState({ listOpen, matching });
  if (seen.listOpen !== listOpen || seen.matching !== matching) {
    setSeen({ listOpen, matching });
    if (!listOpen) setExpanded(false);
    // A search that starts matching, or a list reopened on one that does.
    else if (matching && (!seen.matching || !seen.listOpen)) setExpanded(true);
  }
  return [expanded, setExpanded];
}
