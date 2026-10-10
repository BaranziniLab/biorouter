import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from './command';

/**
 * The highlight contract.
 *
 * The case that matters is the one the section is actually built out of: a
 * picker whose *collection* arrives after it opens, over a couple of fixed
 * action rows that are present from the first paint (`KBSelectorMenu`'s
 * "Manage bases… / Create knowledge base…", `IngestModelPicker`'s provider
 * groups under a fetch). Seeding the highlight against the list that existed at
 * mount and then keeping it because it still *exists* leaves the highlight on
 * an action row, and Enter — the first keystroke, before the user has moved
 * anything — fires that action instead of committing the row they are looking
 * at.
 */

function Harness({
  rows,
  onPick,
  autoFocus = false,
}: {
  rows: string[];
  onPick: (value: string) => void;
  autoFocus?: boolean;
}) {
  const [query, setQuery] = useState('');
  const needle = query.trim().toLowerCase();
  const filtered = needle ? rows.filter((row) => row.toLowerCase().includes(needle)) : rows;

  return (
    <Command label="Rows" query={query} onQueryChange={setQuery}>
      <CommandInput aria-label="Search rows" autoFocus={autoFocus} />
      <CommandList>
        {filtered.length === 0 ? (
          <CommandEmpty>No rows match</CommandEmpty>
        ) : (
          <CommandGroup>
            {filtered.map((row) => (
              <CommandItem key={row} onSelect={() => onPick(row)}>
                {row}
              </CommandItem>
            ))}
          </CommandGroup>
        )}
        <CommandSeparator />
        <CommandGroup>
          <CommandItem onSelect={() => onPick('manage')}>Manage bases…</CommandItem>
          <CommandItem onSelect={() => onPick('create')}>Create knowledge base…</CommandItem>
        </CommandGroup>
      </CommandList>
    </Command>
  );
}

/** The row `aria-activedescendant` currently names, by its visible text. */
function highlighted(): string | null {
  const id = screen.getByRole('combobox').getAttribute('aria-activedescendant');
  if (!id) return null;
  return document.getElementById(id)?.textContent ?? null;
}

describe('Command highlight', () => {
  it('starts on the first row', async () => {
    render(<Harness rows={['Alpha', 'Beta', 'Gamma']} onPick={vi.fn()} />);
    await waitFor(() => expect(highlighted()).toBe('Alpha'));
  });

  // The defect: the collection is still in flight when the picker opens, so the
  // only rows present are the trailing actions and the highlight seeds onto
  // "Manage bases…". When the bases land, that id has not VANISHED — so a
  // repair that only fires on a vanished id keeps it, and the highlight sits on
  // the 4th of 5 rows on a picker the user has not touched yet.
  it('follows the first row when the collection arrives after the picker opens', async () => {
    const { rerender } = render(<Harness rows={[]} onPick={vi.fn()} />);
    await waitFor(() => expect(highlighted()).toBe('Manage bases…'));

    rerender(<Harness rows={['Alpha', 'Beta', 'Gamma']} onPick={vi.fn()} />);
    await waitFor(() => expect(highlighted()).toBe('Alpha'));
  });

  // The same defect stated as its consequence, which is the part that is
  // destructive-feeling: Enter as the FIRST keystroke.
  it('commits the first row on Enter, not the action row the empty list seeded', async () => {
    const onPick = vi.fn();
    const { rerender } = render(<Harness rows={[]} onPick={onPick} autoFocus />);
    await waitFor(() => expect(highlighted()).toBe('Manage bases…'));

    rerender(<Harness rows={['Alpha', 'Beta', 'Gamma']} onPick={onPick} autoFocus />);
    await waitFor(() => expect(highlighted()).toBe('Alpha'));

    screen.getByRole('combobox').focus();
    await userEvent.keyboard('{Enter}');
    expect(onPick).toHaveBeenCalledTimes(1);
    expect(onPick).toHaveBeenCalledWith('Alpha');
  });

  // The other half of the contract: once the user HAS moved the highlight, an
  // unrelated re-render (a poll, a sibling's state change) must not drag it
  // back to the top. A fix that pins the highlight to the first row
  // unconditionally passes the three cases above and breaks the picker.
  it('leaves a highlight the user moved alone across an unrelated re-render', async () => {
    const onPick = vi.fn();
    const { rerender } = render(<Harness rows={['Alpha', 'Beta', 'Gamma']} onPick={onPick} />);
    await waitFor(() => expect(highlighted()).toBe('Alpha'));

    screen.getByRole('combobox').focus();
    await userEvent.keyboard('{ArrowDown}');
    expect(highlighted()).toBe('Beta');

    rerender(<Harness rows={['Alpha', 'Beta', 'Gamma']} onPick={onPick} />);
    await waitFor(() => expect(highlighted()).toBe('Beta'));

    await userEvent.keyboard('{Enter}');
    expect(onPick).toHaveBeenCalledWith('Beta');
  });

  // A row the caller's filter removed is not a row, so the highlight falls back
  // rather than pointing at nothing.
  it('falls back to the first row when the highlighted one is filtered away', async () => {
    render(<Harness rows={['Alpha', 'Beta', 'Gamma']} onPick={vi.fn()} />);
    screen.getByRole('combobox').focus();
    await userEvent.keyboard('{ArrowDown}{ArrowDown}');
    expect(highlighted()).toBe('Gamma');

    await userEvent.type(screen.getByRole('combobox'), 'be');
    await waitFor(() => expect(highlighted()).toBe('Beta'));
  });

  // Typing is a new question, so the answer starts at the top again — even when
  // the row the user had arrowed to survives the filter.
  it('returns the highlight to the top when the query changes', async () => {
    render(<Harness rows={['Alpha', 'Alphabet', 'Alpine']} onPick={vi.fn()} />);
    screen.getByRole('combobox').focus();
    await userEvent.keyboard('{ArrowDown}');
    expect(highlighted()).toBe('Alphabet');

    await userEvent.type(screen.getByRole('combobox'), 'alph');
    await waitFor(() => expect(highlighted()).toBe('Alpha'));
  });

  it('names nothing when every row is gone', async () => {
    render(<Harness rows={[]} onPick={vi.fn()} />);
    // The two action rows are still rows.
    await waitFor(() => expect(highlighted()).toBe('Manage bases…'));
  });
});

/**
 * The listbox keys belong to the field (WS-PICKERS-2). A control inside the root keeps its own
 * Enter and arrows: Enter on a button there used to bubble to the root, be prevented, and click
 * the highlighted row instead.
 */
describe('Command keys stay with the field', () => {
  it('lets Enter on a button inside the root activate that button, not the highlighted row', async () => {
    const onPick = vi.fn();
    const onBulk = vi.fn();
    render(
      <Command label="Bases">
        <CommandInput placeholder="Find a base" />
        <button type="button" onClick={onBulk}>
          Select all
        </button>
        <CommandList>
          <CommandGroup>
            <CommandItem onSelect={() => onPick('Alpha')}>Alpha</CommandItem>
            <CommandItem onSelect={() => onPick('Beta')}>Beta</CommandItem>
          </CommandGroup>
        </CommandList>
      </Command>
    );
    await waitFor(() => expect(highlighted()).toBe('Alpha'));
    screen.getByRole('button', { name: 'Select all' }).focus();
    await userEvent.keyboard('{Enter}');
    expect(onBulk).toHaveBeenCalledTimes(1);
    expect(onPick).not.toHaveBeenCalled();
    // ArrowDown on the button does not move the field's highlight either.
    await userEvent.keyboard('{ArrowDown}');
    expect(highlighted()).toBe('Alpha');
  });

  it('still answers Enter from the field', async () => {
    const onPick = vi.fn();
    render(
      <Command label="Bases">
        <CommandInput placeholder="Find a base" />
        <CommandList>
          <CommandGroup>
            <CommandItem onSelect={() => onPick('Alpha')}>Alpha</CommandItem>
          </CommandGroup>
        </CommandList>
      </Command>
    );
    await waitFor(() => expect(highlighted()).toBe('Alpha'));
    screen.getByRole('combobox').focus();
    await userEvent.keyboard('{Enter}');
    expect(onPick).toHaveBeenCalledWith('Alpha');
  });
});
