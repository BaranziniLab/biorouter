import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ResetProviderSection from './ResetProviderSection';
import { modelsDangerCopy } from '../models/copy';

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

  it('is one Danger zone row: the label, an InfoTip and a destructive Reset… button', () => {
    render(<ResetProviderSection setView={vi.fn()} />);

    // The row label names the button; "Reset…" is only what it says.
    const button = screen.getByRole('button', { name: modelsDangerCopy.row });
    expect(button).toHaveTextContent(modelsDangerCopy.open);
    expect(button).toHaveClass('text-text-danger');
    // The explanation is the InfoTip, reachable from the button itself, never a paragraph.
    expect(button).toHaveAccessibleDescription(modelsDangerCopy.rowHelp);
    expect(
      screen.getByRole('button', { name: `About ${modelsDangerCopy.row}` })
    ).toBeInTheDocument();
    expect(document.querySelector('p.max-w-md')).toBeNull();
  });

  it('asks before resetting, and Cancel changes nothing', async () => {
    const user = userEvent.setup();
    render(<ResetProviderSection setView={vi.fn()} />);

    await user.click(screen.getByRole('button', { name: modelsDangerCopy.row }));
    expect(remove).not.toHaveBeenCalled();
    expect(await screen.findByRole('dialog')).toHaveTextContent(modelsDangerCopy.confirmTitle);

    await user.click(screen.getByRole('button', { name: modelsDangerCopy.cancel }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(remove).not.toHaveBeenCalled();
    expect(reload).not.toHaveBeenCalled();
  });

  it('resets only after the person confirms', async () => {
    const user = userEvent.setup();
    render(<ResetProviderSection setView={vi.fn()} />);

    await user.click(screen.getByRole('button', { name: modelsDangerCopy.row }));
    await user.click(await screen.findByRole('button', { name: modelsDangerCopy.confirm }));

    await waitFor(() => expect(reload).toHaveBeenCalledTimes(1));
    expect(remove).toHaveBeenCalledWith('BIOROUTER_PROVIDER', false);
    expect(remove).toHaveBeenCalledWith('BIOROUTER_MODEL', false);
  });
});
