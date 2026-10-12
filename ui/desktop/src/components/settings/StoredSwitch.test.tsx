import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { StoredSwitch } from './StoredSwitch';

/**
 * A switch for a value read after mount (vision V4-01). Until the value is known it holds its
 * place unpainted; it never shows a guess, and the value, once known, lands without a slide.
 */
describe('StoredSwitch', () => {
  it('holds its place, hidden and disabled, while the value is unknown', () => {
    render(<StoredSwitch checked={null} aria-label="Prevent sleep" />);

    const held = document.querySelector('[data-slot="switch"]') as HTMLElement;
    expect(held).toHaveAttribute('data-pending');
    expect(held).toBeDisabled();
    expect(held.style.visibility).toBe('hidden');
    // It is not on: a placeholder `true` painted the coral track for two frames.
    expect(held).toHaveAttribute('aria-checked', 'false');
    expect(held).toHaveAttribute('data-state', 'unchecked');
    // Unpainted means unreachable too: not in the accessibility tree, so no tab stop and no
    // announcement of a value that is not one.
    expect(screen.queryByRole('switch')).toBeNull();
  });

  it('does not toggle from a click while the value is unknown', () => {
    const onCheckedChange = vi.fn();
    render(
      <StoredSwitch checked={null} aria-label="Prevent sleep" onCheckedChange={onCheckedChange} />
    );
    fireEvent.click(document.querySelector('[data-slot="switch"]') as HTMLElement);
    expect(onCheckedChange).not.toHaveBeenCalled();
  });

  it.each([true, false])('becomes the same switch, showing %s, when the value arrives', (value) => {
    const { rerender } = render(<StoredSwitch checked={null} aria-label="Prevent sleep" />);
    const held = document.querySelector('[data-slot="switch"]') as HTMLElement;

    rerender(<StoredSwitch checked={value} aria-label="Prevent sleep" />);

    const live = screen.getByRole('switch', { name: 'Prevent sleep' });
    expect(live).toBe(held);
    expect(live).not.toHaveAttribute('data-pending');
    expect(live.style.visibility).toBe('');
    expect(live).toBeEnabled();
    expect(live).toHaveAttribute('aria-checked', String(value));
  });

  it('snaps to a value that arrived, instead of sliding the knob there', async () => {
    const { rerender } = render(<StoredSwitch checked={null} aria-label="Prevent sleep" />);
    rerender(<StoredSwitch checked aria-label="Prevent sleep" />);
    // The render that shows the value carries the flag (ui/switch.tsx), so the browser computes
    // the new state with no transition.
    expect(screen.getByRole('switch')).toHaveAttribute('data-motion-still');
    await act(async () => {
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    });
    expect(screen.getByRole('switch')).not.toHaveAttribute('data-motion-still');
  });

  it('keeps a caller-disabled switch disabled once the value is known', () => {
    render(<StoredSwitch checked disabled aria-label="Show in Dock" />);
    expect(screen.getByRole('switch', { name: 'Show in Dock' })).toBeDisabled();
  });
});
