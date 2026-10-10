import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { useRovingRows } from './useRovingRows';

/**
 * The generic copy of Crew's roving rows (spec 2.6): `data-row` / `data-row-item`, one tab stop
 * for the whole list, arrows and Home/End, and the stop returning to the preferred row once
 * focus has left.
 */
const KEYS = ['a', 'b', 'c'];

function List({ preferred = 'b' }: { preferred?: string | null }) {
  const roving = useRovingRows(KEYS, preferred);
  return (
    <>
      <button type="button">before</button>
      <div
        ref={roving.containerRef}
        onKeyDown={roving.onKeyDown}
        onFocus={roving.onFocus}
        onBlur={roving.onBlur}
      >
        {KEYS.map((key) => (
          <div key={key} data-row-item="">
            <button
              type="button"
              data-row={key}
              tabIndex={roving.tabIndexFor(key)}
              onFocus={() => roving.onRowFocus(key)}
            >
              row {key}
            </button>
          </div>
        ))}
      </div>
      <button type="button">after</button>
    </>
  );
}

describe('useRovingRows', () => {
  it('makes the preferred row the one tab stop', () => {
    render(<List />);
    expect(screen.getByRole('button', { name: 'row b' })).toHaveAttribute('tabindex', '0');
    expect(screen.getByRole('button', { name: 'row a' })).toHaveAttribute('tabindex', '-1');
    expect(screen.getByRole('button', { name: 'row c' })).toHaveAttribute('tabindex', '-1');
  });

  it('falls back to the first row when nothing is preferred', () => {
    render(<List preferred={null} />);
    expect(screen.getByRole('button', { name: 'row a' })).toHaveAttribute('tabindex', '0');
  });

  it('moves with the arrows, stops at the ends, and jumps with Home and End', async () => {
    const user = userEvent.setup();
    render(<List />);
    screen.getByRole('button', { name: 'row b' }).focus();
    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('button', { name: 'row c' })).toHaveFocus();
    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('button', { name: 'row c' })).toHaveFocus();
    await user.keyboard('{Home}');
    expect(screen.getByRole('button', { name: 'row a' })).toHaveFocus();
    await user.keyboard('{ArrowUp}');
    expect(screen.getByRole('button', { name: 'row a' })).toHaveFocus();
    await user.keyboard('{End}');
    expect(screen.getByRole('button', { name: 'row c' })).toHaveFocus();
  });

  it('leaves a modified arrow to the browser', () => {
    render(<List />);
    const row = screen.getByRole('button', { name: 'row b' });
    row.focus();
    fireEvent.keyDown(row, { key: 'ArrowDown', shiftKey: true });
    expect(row).toHaveFocus();
  });

  it('keeps the stop on the row that had focus while focus stays inside, then returns it to the preferred row', async () => {
    const user = userEvent.setup();
    render(<List />);
    screen.getByRole('button', { name: 'row b' }).focus();
    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('button', { name: 'row c' })).toHaveAttribute('tabindex', '0');
    expect(screen.getByRole('button', { name: 'row b' })).toHaveAttribute('tabindex', '-1');
    // Tab leaves the list: the next visit starts from the current row again, not the last one.
    await user.tab();
    expect(screen.getByRole('button', { name: 'after' })).toHaveFocus();
    expect(screen.getByRole('button', { name: 'row b' })).toHaveAttribute('tabindex', '0');
    expect(screen.getByRole('button', { name: 'row c' })).toHaveAttribute('tabindex', '-1');
  });
});
