import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HostManagedModelPanel } from './HostManagedModelPanel';
import {
  HOST_CONFIGURE_COMMAND,
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

  it('keeps the title, the reason and the reload line', () => {
    render(<HostManagedModelPanel />);
    const panel = screen.getByTestId('host-managed-model-panel');
    expect(within(panel).getByRole('heading', { level: 2 })).toHaveTextContent(
      HOST_MANAGED_ONBOARDING_TITLE
    );
    expect(panel).toHaveTextContent(HOST_MANAGED_ONBOARDING_REASON);
    expect(panel).toHaveTextContent('Reload this page once that command has finished.');
    // The copy field is the only control the panel holds.
    expect(within(panel).getAllByRole('button')).toHaveLength(1);
  });
});
