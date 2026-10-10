import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ModeSection } from './ModeSection';
import { approvalsCopy } from '../chat/copy';

const config = vi.hoisted(() => ({
  read: vi.fn(),
  upsert: vi.fn(),
  getExtensions: vi.fn(async () => []),
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    read: config.read,
    upsert: config.upsert,
    getExtensions: config.getExtensions,
  }),
}));

function storedMode(mode: string) {
  config.read.mockImplementation(async (key: string) =>
    key === 'BIOROUTER_MODE' ? mode : undefined
  );
}

describe('Settings > Chat > Approvals', () => {
  beforeEach(() => {
    config.read.mockReset();
    config.upsert.mockReset();
    config.upsert.mockResolvedValue(undefined);
  });

  /**
   * One select replaces four radio rows with a paragraph each. The trigger is named by the row
   * label AND the current value, so the visible word is in the name.
   */
  it('shows the stored mode on one select named by its row', async () => {
    storedMode('smart_approve');
    render(<ModeSection />);
    expect(
      await screen.findByRole('button', { name: `${approvalsCopy.mode} Smart` })
    ).toBeInTheDocument();
    expect(screen.queryByRole('radio')).toBeNull();
    expect(screen.queryByRole('radiogroup')).toBeNull();
  });

  it('keeps each mode’s explanation inside its menu item, and saves the choice', async () => {
    storedMode('auto');
    const user = userEvent.setup();
    render(<ModeSection />);
    const trigger = await screen.findByRole('button', { name: `${approvalsCopy.mode} Autonomous` });

    // Not on the page until the menu opens.
    expect(screen.queryByText(approvalsCopy.modes[1].description)).toBeNull();
    await user.click(trigger);
    const manual = await screen.findByRole('menuitemradio', { name: /Manual/ });
    expect(manual).toHaveTextContent(approvalsCopy.modes[1].description);

    await user.click(manual);
    await waitFor(() =>
      expect(config.upsert).toHaveBeenCalledWith('BIOROUTER_MODE', 'approve', false)
    );
    expect(
      await screen.findByRole('button', { name: `${approvalsCopy.mode} Manual` })
    ).toBeInTheDocument();
  });

  /** Tool permissions shape only Manual and Smart, so the one Edit button is live only there. */
  it.each([
    ['auto', false],
    ['approve', true],
    ['smart_approve', true],
    ['chat', false],
  ])('in %s mode, Edit tool permissions is enabled: %s', async (mode, enabled) => {
    storedMode(mode);
    render(<ModeSection />);
    await screen.findByRole('button', {
      name: `${approvalsCopy.mode} ${approvalsCopy.modes.find((m) => m.key === mode)!.label}`,
    });
    const edit = screen.getByRole('button', { name: approvalsCopy.editToolPermissions });
    if (enabled) expect(edit).toBeEnabled();
    else expect(edit).toBeDisabled();
  });

  it('opens the tool permission rules from the row', async () => {
    storedMode('approve');
    const user = userEvent.setup();
    render(<ModeSection />);
    await screen.findByRole('button', { name: `${approvalsCopy.mode} Manual` });
    await user.click(screen.getByRole('button', { name: approvalsCopy.editToolPermissions }));
    expect(await screen.findByRole('dialog')).toHaveTextContent('Tool permissions');
  });

  it('shows Max turns without a disclosure', async () => {
    storedMode('auto');
    render(<ModeSection />);
    expect(await screen.findByRole('spinbutton', { name: approvalsCopy.maxTurns })).toBeVisible();
    expect(screen.queryByRole('button', { name: /chat limits/i })).toBeNull();
  });
});
