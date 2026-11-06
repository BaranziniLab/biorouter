import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { generalCopy } from './copy';

const mocks = vi.hoisted(() => ({ upsert: vi.fn(), read: vi.fn() }));

vi.mock('../../ConfigContext', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>;
  return {
    ...actual,
    useConfig: () => ({ upsert: mocks.upsert, read: mocks.read }),
  };
});

import { OPEN_SUBAGENT_TABS_KEY, OpenSubagentTabsRow } from './WorkspaceSettingsSection';

/**
 * The key the daemon reads; its default is OFF (subagents run in the background). The last
 * answer lives in `../lastKnown.ts` like every sibling switch's, which `src/test/setup.ts` forgets
 * after each test. Its place in the General rows is asserted in `AppSettingsSection.test.tsx`.
 */
describe('Open subagent tabs automatically', () => {
  afterEach(() => vi.clearAllMocks());

  it('reads and writes BIOROUTER_WORKSPACE_OPEN_SUBAGENT_TABS, defaulting to off', async () => {
    expect(OPEN_SUBAGENT_TABS_KEY).toBe('BIOROUTER_WORKSPACE_OPEN_SUBAGENT_TABS');
    mocks.read.mockResolvedValue(false);
    render(<OpenSubagentTabsRow />);
    const toggle = await screen.findByRole('switch', { name: generalCopy.openSubagentTabs });
    expect(mocks.read).toHaveBeenCalledWith(OPEN_SUBAGENT_TABS_KEY, false);
    expect(toggle).not.toBeChecked();
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(mocks.upsert).toHaveBeenCalledWith(OPEN_SUBAGENT_TABS_KEY, true, false)
    );
    expect(toggle).toBeChecked();
  });

  it('starts checked when the key is already true, and explains itself as help', async () => {
    mocks.read.mockResolvedValue(true);
    render(<OpenSubagentTabsRow />);
    const toggle = await screen.findByRole('switch', { name: generalCopy.openSubagentTabs });
    await waitFor(() => expect(toggle).toBeChecked());
    expect(toggle).toHaveAccessibleDescription(generalCopy.openSubagentTabsHelp);
  });

  /**
   * "Never open tabs automatically" wins on the daemon (`resolve_visibility`): with it on, no
   * subagent tab opens whatever this switch says. The switch is not disabled for it, so its help
   * is what tells the person.
   */
  it('says in its help that "Never open tabs automatically" wins', () => {
    expect(generalCopy.openSubagentTabsHelp).toContain(generalCopy.announceOnly);
  });

  it('rolls the switch back when the write fails', async () => {
    mocks.read.mockResolvedValue(false);
    mocks.upsert.mockRejectedValueOnce(new Error('refused'));
    render(<OpenSubagentTabsRow />);
    const toggle = await screen.findByRole('switch', { name: generalCopy.openSubagentTabs });
    fireEvent.click(toggle);
    await waitFor(() => expect(mocks.upsert).toHaveBeenCalled());
    await waitFor(() => expect(toggle).not.toBeChecked());
  });

  it('holds the switch unpainted until the daemon answers, and shows off when it cannot', async () => {
    mocks.read.mockImplementation(() => new Promise(() => {}));
    const { unmount } = render(<OpenSubagentTabsRow />);
    expect(document.querySelector('[data-slot="switch"]')).toHaveAttribute('data-pending');
    await act(async () => {});
    unmount();
    mocks.read.mockRejectedValue(new Error('daemon down'));
    render(<OpenSubagentTabsRow />);
    expect(
      await screen.findByRole('switch', { name: generalCopy.openSubagentTabs })
    ).not.toBeChecked();
  });

  it('starts a later visit from the last answer, painted in the first render', async () => {
    mocks.read.mockResolvedValue(true);
    const first = render(<OpenSubagentTabsRow />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: generalCopy.openSubagentTabs })).toBeChecked()
    );
    first.unmount();
    mocks.read.mockImplementation(() => new Promise(() => {}));
    render(<OpenSubagentTabsRow />);
    expect(screen.getByRole('switch', { name: generalCopy.openSubagentTabs })).toBeChecked();
    await act(async () => {});
  });
});
