import * as React from 'react';
import { Avatar } from '../../ui/avatar';
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from '../../ui/command';
import { Popover, PopoverContent, PopoverTrigger } from '../../ui/popover';
import { ChevronDown } from '../../icons/app-icons';
import { cn } from '../../../utils';
import { PersonName, type CrewPerson, type PeopleDirectory } from '../identity';
import { addPeopleCopy } from './copy';
import { personMatches } from './people';

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
      <PopoverContent align="start" className="w-80 p-0">
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
