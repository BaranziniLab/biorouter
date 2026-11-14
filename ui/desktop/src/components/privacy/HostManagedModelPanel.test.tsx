import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HostManagedModelPanel } from './HostManagedModelPanel';
import {
  HOST_CONFIGURE_COMMAND,
  HOST_MANAGED_ONBOARDING_HELP,
  HOST_MANAGED_ONBOARDING_REASON,
  HOST_MANAGED_ONBOARDING_TITLE,
} from './hostManagedModelCopy';

/**
 * Ruling 25: the command a browser user must run on the host is the shared `CopyField`, as every
 * other command on first run is, and nothing else in the panel moves.
 */
describe('HostManagedModelPanel', () => {
  let writeText: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
  });

  afterEach(() => {
    writeText.mockRestore();
  });

  it('offers the host command in the shared copy field', async () => {
    render(<HostManagedModelPanel />);
    const panel = screen.getByTestId('host-managed-model-panel');

    const field = panel.querySelector('[data-slot="copy-field"]') as HTMLElement | null;
    expect(field).not.toBeNull();
    expect(field).toHaveTextContent(HOST_CONFIGURE_COMMAND);
    // The bare block it replaced, with no way to copy, is gone.
    expect(panel.querySelector('pre')).toBeNull();

    const copy = within(panel).getByRole('button', { name: `Copy ${HOST_CONFIGURE_COMMAND}` });
    await act(async () => {
      fireEvent.click(copy);
    });
    expect(writeText).toHaveBeenCalledWith(HOST_CONFIGURE_COMMAND);
  });

  it('keeps the title, one visible line and the command, with the rest in an InfoTip', () => {
    render(<HostManagedModelPanel />);
    const panel = screen.getByTestId('host-managed-model-panel');
    // The heading is named by the title alone: the info icon sits beside it, not inside.
    expect(within(panel).getByRole('heading', { level: 2 })).toHaveTextContent(
      new RegExp(`^${HOST_MANAGED_ONBOARDING_TITLE}$`)
    );
    expect(within(panel).getByTestId('host-managed-model-reason')).toHaveTextContent(
      HOST_MANAGED_ONBOARDING_REASON
    );
    // What to do and why the choice stays on the host: the title's info icon.
    const about = within(panel).getByRole('button', { name: 'About choosing a model on the host' });
    expect(about).toHaveAccessibleDescription(HOST_MANAGED_ONBOARDING_HELP);
    // The copy field and that info icon are the only controls the panel holds.
    expect(within(panel).getAllByRole('button')).toHaveLength(2);
  });

  // SD-18: the daemon knows why it refused this browser, so its sentence replaces the default.
  it('shows the sentence the daemon gave in place of the default line', () => {
    render(<HostManagedModelPanel reason="Only the person who started this server can choose." />);
    const line = screen.getByTestId('host-managed-model-reason');
    expect(line).toHaveTextContent('Only the person who started this server can choose.');
    expect(line).not.toHaveTextContent(HOST_MANAGED_ONBOARDING_REASON);
    expect(screen.getByTestId('host-managed-model-panel')).toHaveTextContent(
      HOST_CONFIGURE_COMMAND
    );
  });
});
