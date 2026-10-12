import { describe, expect, it, vi, afterEach } from 'vitest';
import { act, render, screen, fireEvent, waitFor } from '@testing-library/react';
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

  /**
   * A switch for a setting the daemon holds is the same control until it answers, held unpainted,
   * and a later visit starts from the last answer (vision round 8, V4-R7-2): a placeholder `false`
   * drew the OFF knob for a person who had turned this on, and slid it across on every visit.
   */
  describe('before and after the daemon has answered', () => {
    it('holds the switch unpainted on a first visit rather than painting an OFF it does not know', async () => {
      mocks.read.mockImplementation(() => new Promise(() => {}));
      render(<NeverOpenTabsRow />);
      expect(document.querySelector('[data-slot="switch"]')).toHaveAttribute('data-pending');
      expect(screen.queryByRole('switch', { name: generalCopy.announceOnly })).toBeNull();
      await act(async () => {});
    });

    it('starts a later visit from the last answer, painted in the first render', async () => {
      mocks.read.mockResolvedValue(true);
      const first = render(<NeverOpenTabsRow />);
      await waitFor(() =>
        expect(screen.getByRole('switch', { name: generalCopy.announceOnly })).toBeChecked()
      );
      first.unmount();

      mocks.read.mockImplementation(() => new Promise(() => {}));
      render(<NeverOpenTabsRow />);
      expect(document.querySelector('[data-slot="switch"]')).not.toHaveAttribute('data-pending');
      expect(screen.getByRole('switch', { name: generalCopy.announceOnly })).toBeChecked();
      await act(async () => {});
    });

    it('shows the default, off, when the key cannot be read and nothing was ever read', async () => {
      mocks.read.mockRejectedValue(new Error('daemon down'));
      render(<NeverOpenTabsRow />);
      expect(
        await screen.findByRole('switch', { name: generalCopy.announceOnly })
      ).not.toBeChecked();
    });
  });
});
