import { useState } from 'react';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { SegmentedControl, type SegmentedOption } from './segmented-control';

type Mode = 'light' | 'dark' | 'system';
const OPTIONS: SegmentedOption<Mode>[] = [
  { value: 'light', label: 'Light', testId: 'light-mode-button' },
  { value: 'dark', label: 'Dark', testId: 'dark-mode-button' },
  { value: 'system', label: 'System', testId: 'system-mode-button' },
];

function Controlled({ fill, onChange }: { fill?: boolean; onChange?: (value: Mode) => void }) {
  const [value, setValue] = useState<Mode>('light');
  return (
    <div>
      <button type="button">Before</button>
      <SegmentedControl
        aria-label="Theme"
        options={OPTIONS}
        value={value}
        fill={fill}
        onValueChange={(next) => {
          setValue(next);
          onChange?.(next);
        }}
      />
      <button type="button">After</button>
    </div>
  );
}

describe('SegmentedControl', () => {
  it('is a radiogroup of radios with aria-checked, keeping each testid', () => {
    render(<Controlled />);
    const group = screen.getByRole('radiogroup', { name: 'Theme' });
    expect(group).toHaveClass('br-segmented');
    const radios = screen.getAllByRole('radio');
    expect(radios).toHaveLength(3);
    expect(screen.getByTestId('light-mode-button')).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByTestId('dark-mode-button')).toHaveAttribute('aria-checked', 'false');
  });

  it('is one Tab stop', async () => {
    const user = userEvent.setup();
    render(<Controlled />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(screen.getByRole('radio', { name: 'Light' })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole('button', { name: 'After' })).toHaveFocus();
  });

  it('moves and selects with the arrow keys, and jumps with Home and End', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Controlled onChange={onChange} />);
    act(() => screen.getByRole('radio', { name: 'Light' }).focus());
    // Held, as a person's key is: Radix selects on the focus an arrow key moves.
    await user.keyboard('{ArrowRight>}');
    await waitFor(() =>
      expect(screen.getByRole('radio', { name: 'Dark' })).toHaveAttribute('aria-checked', 'true')
    );
    await user.keyboard('{/ArrowRight}');
    await user.keyboard('{End}');
    expect(onChange).toHaveBeenLastCalledWith('system');
    expect(screen.getByRole('radio', { name: 'System' })).toHaveAttribute('aria-checked', 'true');
    await user.keyboard('{Home}');
    expect(onChange).toHaveBeenLastCalledWith('light');
  });

  it('selects on click', async () => {
    const user = userEvent.setup();
    render(<Controlled />);
    await user.click(screen.getByRole('radio', { name: 'System' }));
    expect(screen.getByRole('radio', { name: 'System' })).toHaveAttribute('aria-checked', 'true');
  });

  it('draws one thumb, unplaced until measured (jsdom measures 0), and never accent', () => {
    render(<Controlled />);
    const thumbs = document.querySelectorAll('[data-slot="segmented-thumb"]');
    expect(thumbs).toHaveLength(1);
    expect(thumbs[0]).toHaveAttribute('aria-hidden', 'true');
    // Fit mode in jsdom: no width to measure, so the selected segment paints its own ground.
    expect(thumbs[0]).toHaveAttribute('data-placed', 'false');
    expect(screen.getByRole('radiogroup')).toHaveAttribute('data-placed', 'false');
    expect(thumbs[0].className).not.toMatch(/accent/);
  });

  it('fill gives equal columns and a computed thumb', () => {
    render(<Controlled fill />);
    const group = screen.getByRole('radiogroup');
    expect(group).toHaveAttribute('data-fill', 'true');
    const thumb = document.querySelector<HTMLElement>('[data-slot="segmented-thumb"]')!;
    expect(thumb).toHaveAttribute('data-placed', 'true');
    expect(thumb.style.getPropertyValue('--br-segmented-count')).toBe('3');
    expect(thumb.style.getPropertyValue('--br-segmented-index')).toBe('0');
  });

  it('places the first thumb without motion', () => {
    render(<Controlled fill />);
    const thumb = document.querySelector('[data-slot="segmented-thumb"]')!;
    expect(thumb).toHaveAttribute('data-motion-still');
  });

  it('names an icon-only segment by its ariaLabel', () => {
    render(
      <SegmentedControl
        aria-label="View"
        value="a"
        onValueChange={() => {}}
        options={[
          { value: 'a', label: 'P', ariaLabel: 'Preview' },
          { value: 'b', label: 'R', ariaLabel: 'Raw' },
        ]}
      />
    );
    expect(screen.getByRole('radio', { name: 'Preview' })).toBeInTheDocument();
  });
});
