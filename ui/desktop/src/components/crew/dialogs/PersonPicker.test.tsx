import { fireEvent, render, screen, within } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { buildPeopleDirectory, type CrewPerson } from '../identity';
import { addPeopleCopy } from './copy';
import { alice, bob, carol, dan, makeSnapshot } from './dialogsTestHarness';
import { PersonChecklist } from './PersonPicker';

/**
 * The Add people checklist (QA Q2-05): search, "Select all ({n})" for the people shown, and a real
 * checkbox per person named in the authority form.
 */

const dir = buildPeopleDirectory(makeSnapshot());
const people = [bob, carol, dan].map((principal) => dir.byId(principal.id) as CrewPerson);

function Harness({
  candidates = people,
  onChange,
}: {
  candidates?: CrewPerson[];
  onChange?: (ids: string[]) => void;
}) {
  const [selected, setSelected] = useState<string[]>([]);
  return (
    <>
      <span id="people-label">People</span>
      <PersonChecklist
        candidates={candidates}
        selected={selected}
        onChange={(ids) => {
          setSelected(ids);
          onChange?.(ids);
        }}
        label="People"
        labelledBy="people-label"
        dir={dir}
      />
      <output data-testid="selected">{[...selected].sort().join(',')}</output>
    </>
  );
}

const selected = () => screen.getByTestId('selected').textContent;
const search = () => screen.getByRole('searchbox', { name: addPeopleCopy.search });
const selectAll = (count: number) =>
  screen.getByRole('checkbox', { name: addPeopleCopy.selectAll(count) });

describe('PersonChecklist', () => {
  it('names every row in full, display name and @username, as its checkbox’s name', () => {
    render(<Harness />);
    const list = screen.getByRole('list', { name: 'People' });
    expect(
      within(list)
        .getAllByRole('checkbox')
        .map((box) => box.getAttribute('type'))
    ).toEqual(['checkbox', 'checkbox', 'checkbox']);
    for (const name of ['Bob Lee (@bob)', 'Carol Diaz (@carol)', 'Dan Wu (@dan)'])
      expect(within(list).getByRole('checkbox', { name })).not.toBeChecked();
    expect(alice.id).toBe('person-alice');
  });

  it('ticks everyone shown with Select all, and clears them the same way', () => {
    render(<Harness />);
    fireEvent.click(selectAll(3));
    expect(selected()).toBe('person-bob,person-carol,person-dan');
    expect(selectAll(3)).toBeChecked();
    fireEvent.click(selectAll(3));
    expect(selected()).toBe('');
  });

  it('shows a partial choice as mixed', () => {
    render(<Harness />);
    fireEvent.click(screen.getByRole('checkbox', { name: 'Carol Diaz (@carol)' }));
    expect(selected()).toBe('person-carol');
    const all = selectAll(3) as HTMLInputElement;
    expect(all.indeterminate).toBe(true);
    expect(all).not.toBeChecked();
  });

  it('selects only the people a search shows, and keeps a choice the search hides', () => {
    render(<Harness />);
    fireEvent.click(screen.getByRole('checkbox', { name: 'Bob Lee (@bob)' }));
    fireEvent.change(search(), { target: { value: 'd' } });
    // "d": Carol Diaz and Dan Wu.
    fireEvent.click(selectAll(2));
    expect(selected()).toBe('person-bob,person-carol,person-dan');
    fireEvent.click(selectAll(2));
    // Bob, hidden by the search, is still chosen.
    expect(selected()).toBe('person-bob');
    fireEvent.change(search(), { target: { value: 'zed' } });
    expect(screen.getByText(addPeopleCopy.noMatch('zed'))).toBeInTheDocument();
    expect(screen.queryByRole('checkbox', { name: /^Select all/ })).toBeNull();
  });

  it('offers no Select all for one person', () => {
    const onChange = vi.fn();
    render(<Harness candidates={[people[0]]} onChange={onChange} />);
    expect(screen.queryByRole('checkbox', { name: /^Select all/ })).toBeNull();
    fireEvent.click(screen.getByRole('checkbox', { name: 'Bob Lee (@bob)' }));
    expect(onChange).toHaveBeenLastCalledWith(['person-bob']);
  });
});
