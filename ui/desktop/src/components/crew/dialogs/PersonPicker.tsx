import * as React from 'react';
import { Avatar } from '../../ui/avatar';
import { Checkbox } from '../../ui/Checkbox';
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from '../../ui/command';
import { Input } from '../../ui/input';
import { Popover, PopoverContent, PopoverTrigger } from '../../ui/popover';
import { ChevronDown } from '../../icons/app-icons';
import { cn } from '../../../utils';
import { PersonName, type CrewPerson, type PeopleDirectory } from '../identity';
import { addPeopleCopy } from './copy';
import { personMatches } from './people';
import './dialogs.css';

export interface PersonPickerProps {
  /** Who may be chosen. The caller has already removed members and pending invitees. */
  candidates: readonly CrewPerson[];
  /** The chosen principal ID, or null. */
  value: string | null;
  onChange(principalId: string | null): void;
  /** The field's label in words, for the listbox. */
  label: string;
  /** The id of the field's visible label; the trigger is named by it and by the choice. */
  labelledBy: string;
  /** The trigger's id. */
  id?: string;
  /** Shown in the trigger while nothing is chosen. */
  placeholder?: string;
  /** Shown instead of the list when there are no candidates at all. */
  emptyText?: string;
  dir?: PeopleDirectory | null;
  disabled?: boolean;
  /** Focus the field when it mounts — for a dialog step that replaced the field that had focus. */
  autoFocus?: boolean;
  'aria-describedby'?: string;
}

/**
 * Choosing a person for an authority-bearing action: add to a team or channel, offer ownership.
 *
 * A `Popover` + `Command` listbox, never a menu with a field in it (`command.tsx` says why). Every
 * row shows the person in the authority form — display name AND `@username`, both in full — before
 * it can be chosen, so a nickname that imitates someone cannot pass for them (naming design,
 * "Selectors and the resolver", rule 3). Search matches display names and usernames; the choice
 * is a principal ID, which the caller sends with `expected_username` so the broker refuses a
 * mismatch.
 */
export function PersonPicker({
  candidates,
  value,
  onChange,
  label,
  labelledBy,
  id,
  placeholder = addPeopleCopy.choose,
  emptyText,
  dir,
  disabled,
  autoFocus,
  'aria-describedby': describedBy,
}: PersonPickerProps) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState('');
  const valueId = React.useId();
  const chosen = candidates.find((person) => person.id === value) ?? null;
  const visible = candidates.filter((person) => personMatches(person, query));

  const choose = (person: CrewPerson) => {
    onChange(person.id);
    setOpen(false);
    setQuery('');
  };

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) setQuery('');
      }}
    >
      <PopoverTrigger asChild>
        <button
          id={id}
          type="button"
          disabled={disabled}
          autoFocus={autoFocus}
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-labelledby={`${labelledBy} ${valueId}`}
          aria-describedby={describedBy}
          data-person-picker=""
          className="biorouter-focus-surface flex h-control-md w-full min-w-0 items-center gap-2 rounded-element border border-border-emphasized bg-background-default px-2 text-left text-label hover:inset-ring-2 hover:inset-ring-border-emphasized/30 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {chosen ? (
            <>
              <Avatar
                size={20}
                fallback={chosen.avatar}
                name={chosen.displayName}
                username={chosen.username}
              />
              <span id={valueId} className="min-w-0 flex-1 truncate">
                <PersonName person={chosen} context="authority" dir={dir} />
              </span>
            </>
          ) : (
            <span id={valueId} className="min-w-0 flex-1 truncate text-text-muted">
              {placeholder}
            </span>
          )}
          <ChevronDown
            aria-hidden
            className={cn(
              'h-icon-row w-icon-row shrink-0 text-text-muted transition-transform',
              open && 'rotate-180'
            )}
          />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="crew-person-picker-list p-0">
        <Command label={label} query={query} onQueryChange={setQuery} className="max-h-72">
          <CommandInput placeholder={addPeopleCopy.search} aria-label={addPeopleCopy.search} />
          <CommandList aria-label={label}>
            {visible.map((person) => (
              <CommandItem
                key={person.id ?? person.username}
                selected={person.id === value}
                onSelect={() => choose(person)}
              >
                <Avatar
                  size={20}
                  fallback={person.avatar}
                  name={person.displayName}
                  username={person.username}
                />
                <PersonName
                  person={person}
                  context="authority"
                  dir={dir}
                  className="min-w-0 flex-1 truncate"
                />
              </CommandItem>
            ))}
            {visible.length === 0 ? (
              <CommandEmpty className="text-supporting text-text-muted">
                {candidates.length === 0 && emptyText
                  ? emptyText
                  : addPeopleCopy.noMatch(query.trim())}
              </CommandEmpty>
            ) : null}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

export interface PersonChecklistProps {
  /** Who may be chosen. The caller has already removed members and pending invitees. */
  candidates: readonly CrewPerson[];
  /** The chosen principal IDs, in any order. Kept through a search that hides some of them. */
  selected: readonly string[];
  onChange(principalIds: string[]): void;
  /** The group's name in words (its visible label is the caller's `labelledBy`). */
  label: string;
  /** The id of the visible label naming the list. */
  labelledBy?: string;
  dir?: PeopleDirectory | null;
  disabled?: boolean;
}

/**
 * Choosing several people at once for Add people (QA Q2-05: four people took sixteen interactions,
 * one dialog each, and a lab of twenty would take eighty). A search box, "Select all ({n})" for the
 * people it shows, and one real checkbox per person, each wrapped in its label so the whole row
 * toggles it.
 *
 * Every row shows the person in the authority form — display name AND `@username`, in full —
 * before they can be ticked, as `PersonPicker` does, so a nickname that imitates someone cannot pass
 * for them. The choice is principal IDs; the caller sends each with its `expected_username`, and
 * the broker decides whether the viewer may add them.
 */
export function PersonChecklist({
  candidates,
  selected,
  onChange,
  label,
  labelledBy,
  dir,
  disabled,
}: PersonChecklistProps) {
  const [query, setQuery] = React.useState('');
  const listId = React.useId();
  const chosen = new Set(selected);
  const visible = candidates.filter((person) => person.id && personMatches(person, query));
  const visibleIds = visible.map((person) => person.id as string);
  const allShown = visibleIds.length > 0 && visibleIds.every((id) => chosen.has(id));
  const someShown = visibleIds.some((id) => chosen.has(id));

  const toggle = (id: string, next: boolean) =>
    onChange(
      next
        ? [...selected.filter((item) => item !== id), id]
        : selected.filter((item) => item !== id)
    );
  const toggleShown = (next: boolean) =>
    onChange(
      next
        ? [...selected, ...visibleIds.filter((id) => !chosen.has(id))]
        : selected.filter((id) => !visibleIds.includes(id))
    );

  return (
    <div className="flex min-w-0 flex-col gap-2">
      <Input
        type="search"
        placeholder={addPeopleCopy.search}
        aria-label={addPeopleCopy.search}
        aria-controls={listId}
        autoComplete="off"
        spellCheck={false}
        disabled={disabled}
        value={query}
        onChange={(event) => setQuery(event.target.value)}
      />
      {visible.length > 1 ? (
        <label className="flex min-w-0 items-center gap-2 px-1 text-label text-text-default">
          <Checkbox
            checked={allShown}
            indeterminate={!allShown && someShown}
            disabled={disabled}
            onChange={(event) => toggleShown(event.target.checked)}
          />
          <span>{addPeopleCopy.selectAll(visible.length)}</span>
        </label>
      ) : null}
      <ul
        id={listId}
        role="list"
        aria-labelledby={labelledBy}
        aria-label={labelledBy ? undefined : label}
        className="crew-person-checklist flex min-w-0 flex-col"
      >
        {visible.map((person) => (
          <li key={person.id}>
            <label className="flex min-w-0 items-center gap-2 px-1 py-1 text-label">
              <Checkbox
                checked={chosen.has(person.id as string)}
                disabled={disabled}
                onChange={(event) => toggle(person.id as string, event.target.checked)}
              />
              <Avatar
                size={20}
                fallback={person.avatar}
                name={person.displayName}
                username={person.username}
              />
              <PersonName
                person={person}
                context="authority"
                dir={dir}
                className="min-w-0 flex-1 truncate"
              />
            </label>
          </li>
        ))}
      </ul>
      {visible.length === 0 ? (
        <p role="status" className="px-1 text-supporting text-text-muted">
          {addPeopleCopy.noMatch(query.trim())}
        </p>
      ) : null}
    </div>
  );
}
