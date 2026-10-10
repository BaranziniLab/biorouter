import { describe, expect, it, vi, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { generalCopy } from './copy';

const mocks = vi.hoisted(() => ({ upsert: vi.fn(), read: vi.fn() }));

vi.mock('../../ConfigContext', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>;
  return {
    ...actual,
    useConfig: () => ({ upsert: mocks.upsert, read: mocks.read }),
  };
});

import { NeverOpenTabsRow } from './WorkspaceSettingsSection';

describe('Never open tabs automatically', () => {
  afterEach(() => vi.clearAllMocks());

  it('reflects the stored value and writes the config key on toggle', async () => {
    mocks.read.mockResolvedValue(false);
    render(<NeverOpenTabsRow />);
    const toggle = await screen.findByRole('switch', { name: generalCopy.announceOnly });
    expect(toggle.getAttribute('aria-checked')).toBe('false');

    fireEvent.click(toggle);
    await waitFor(() =>
      expect(mocks.upsert).toHaveBeenCalledWith('WORKSPACE_ANNOUNCE_ONLY', true, false)
    );
  });

  it('starts checked when the key is already true', async () => {
    mocks.read.mockResolvedValue(true);
    render(<NeverOpenTabsRow />);
    const toggle = await screen.findByRole('switch', { name: generalCopy.announceOnly });
    await waitFor(() => expect(toggle.getAttribute('aria-checked')).toBe('true'));
  });

  it('rolls the switch back when the write fails', async () => {
    mocks.read.mockResolvedValue(false);
    mocks.upsert.mockRejectedValueOnce(new Error('refused'));
    render(<NeverOpenTabsRow />);
    const toggle = await screen.findByRole('switch', { name: generalCopy.announceOnly });
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle.getAttribute('aria-checked')).toBe('false'));
  });

  /**
   * It used to be a whole "Workspace" section holding this one row, with a two-sentence
   * paragraph under the label. It is a row of General now, and the explanation is help the
   * switch hears through `aria-describedby` rather than a paragraph on the page.
   */
  it('is one row with its explanation as help, not a section with a paragraph', async () => {
    mocks.read.mockResolvedValue(false);
    const { container } = render(<NeverOpenTabsRow />);
    const toggle = await screen.findByRole('switch', { name: generalCopy.announceOnly });

    expect(container.querySelector('.biorouter-settings-section')).toBeNull();
    expect(screen.queryByRole('heading')).toBeNull();
    expect(container.querySelector('.biorouter-settings-row p')).toBeNull();
    expect(toggle).toHaveAccessibleDescription(generalCopy.announceOnlyHelp);
  });
});
