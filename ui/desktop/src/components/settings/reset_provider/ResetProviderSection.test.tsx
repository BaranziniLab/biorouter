import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ResetProviderSection from './ResetProviderSection';

const remove = vi.hoisted(() => vi.fn(async () => undefined));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ remove }),
}));
vi.mock('../../../utils/surface', () => ({ isBrowserSurface: () => false }));

// W2-PRV-11: "Reset provider and model" removed both keys and reloaded the
// window on its first click.
describe('ResetProviderSection', () => {
  const reload = vi.fn();
  const originalLocation = window.location;

  beforeEach(() => {
    remove.mockClear();
    reload.mockClear();
    Object.defineProperty(window, 'location', {
      configurable: true,
      value: { ...originalLocation, reload },
    });
  });
  afterEach(() => {
    Object.defineProperty(window, 'location', { configurable: true, value: originalLocation });
  });

  it('asks before resetting, and Cancel changes nothing', async () => {
    const user = userEvent.setup();
    render(<ResetProviderSection setView={vi.fn()} />);

    await user.click(screen.getByRole('button', { name: /reset provider and model/i }));
    expect(remove).not.toHaveBeenCalled();
    expect(await screen.findByRole('dialog')).toHaveTextContent('Reset provider and model?');

    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(remove).not.toHaveBeenCalled();
    expect(reload).not.toHaveBeenCalled();
  });

  it('resets only after the person confirms', async () => {
    const user = userEvent.setup();
    render(<ResetProviderSection setView={vi.fn()} />);

    await user.click(screen.getByRole('button', { name: /reset provider and model/i }));
    await user.click(await screen.findByRole('button', { name: 'Reset' }));

    await waitFor(() => expect(reload).toHaveBeenCalledTimes(1));
    expect(remove).toHaveBeenCalledWith('BIOROUTER_PROVIDER', false);
    expect(remove).toHaveBeenCalledWith('BIOROUTER_MODEL', false);
  });
});
