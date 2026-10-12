import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import { ModalShell } from '../ModalShell';
import { Button } from './button';
import { Select } from './Select';

/**
 * A pick leaves focus on the Select (vision round 8, V3-R8-2).
 *
 * In New schedule, ArrowDown, ArrowUp and Enter on "AM or PM" picked AM and then moved focus to
 * the dialog box: the wrapper forced react-select's `blurInputOnSelect` on, the blurred field let
 * Radix's focus trap park focus on the dialog surface (no ring, a quiet open), and the next Tab
 * restarted at Name instead of reaching Cancel. A mouse pick did the same. This mirrors that form
 * in the real `ModalShell`: a text field, then a non-searchable Select, then the footer.
 */
const PERIODS = [
  { value: 'AM', label: 'AM' },
  { value: 'PM', label: 'PM' },
];

function NewSchedule({ blurInputOnSelect }: { blurInputOnSelect?: boolean }) {
  const [period, setPeriod] = useState('PM');
  return (
    <ModalShell
      open
      onOpenChange={() => {}}
      purpose="form"
      title="New schedule"
      footer={
        <>
          <Button variant="secondary">Cancel</Button>
          <Button>Create</Button>
        </>
      }
    >
      <label htmlFor="schedule-name">Name</label>
      <input id="schedule-name" />
      <Select
        aria-label="AM or PM"
        isSearchable={false}
        options={PERIODS}
        value={PERIODS.find((option) => option.value === period)}
        onChange={(next: unknown) => setPeriod((next as { value: string }).value)}
        {...(blurInputOnSelect === undefined ? {} : { blurInputOnSelect })}
      />
      <output data-testid="period">{period}</output>
    </ModalShell>
  );
}

const field = () => screen.getByRole('combobox', { name: 'AM or PM' });

describe('a Select pick keeps focus on the Select (V3-R8-2)', () => {
  it('keyboard: ArrowDown, ArrowUp, Enter picks AM, focus stays, and Tab reaches Cancel', async () => {
    const user = userEvent.setup();
    render(<NewSchedule />);

    await user.click(screen.getByLabelText('Name'));
    await user.tab();
    expect(field()).toHaveFocus();
    await user.keyboard('{ArrowDown}');
    expect(await screen.findAllByRole('option')).toHaveLength(2);
    await user.keyboard('{ArrowUp}{Enter}');

    expect(screen.getByTestId('period')).toHaveTextContent('AM');
    expect(screen.queryAllByRole('option')).toHaveLength(0);
    expect(field()).toHaveFocus();
    expect(field()).toHaveAttribute('aria-expanded', 'false');

    await user.tab();
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();
  });

  it('pointer: a picked option leaves focus on the Select, not on the dialog box', async () => {
    const user = userEvent.setup();
    render(<NewSchedule />);

    await user.click(field());
    await user.keyboard('{ArrowDown}');
    await user.click(await screen.findByRole('option', { name: 'AM' }));

    expect(screen.getByTestId('period')).toHaveTextContent('AM');
    await waitFor(() => expect(screen.queryAllByRole('option')).toHaveLength(0));
    expect(field()).toHaveFocus();
    expect(screen.getByRole('dialog')).not.toHaveFocus();
  });

  it('a caller can still ask for the blur (the old default, the instrument)', async () => {
    const user = userEvent.setup();
    render(<NewSchedule blurInputOnSelect />);

    await user.click(screen.getByLabelText('Name'));
    await user.tab();
    await user.keyboard('{ArrowDown}');
    expect(await screen.findAllByRole('option')).toHaveLength(2);
    await user.keyboard('{ArrowUp}{Enter}');

    expect(screen.getByTestId('period')).toHaveTextContent('AM');
    expect(field()).not.toHaveFocus();
  });
});
