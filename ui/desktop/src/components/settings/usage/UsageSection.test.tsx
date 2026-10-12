import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { getUsageReport, getUsageSummary, type UsageSummaryResponse } from '../../../api';
import { settingsUsageCopy as copy } from '../../usage/copy';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import UsageSection from './UsageSection';
import { DATA_RESET_EVENT, forgetUsageSnapshot, rememberUsageSnapshot } from './usageSnapshot';

vi.mock('../../../api', () => ({
  getUsageReport: vi.fn(),
  getUsageSummary: vi.fn(),
}));

/** The month-to-date summary these tests serve; `turns` tells one answer from another. */
const summaryBody = (turns: number): UsageSummaryResponse => {
  const totals = {
    inputTokens: 0,
    outputTokens: 0,
    totalTokens: 0,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    turns,
    cost: 0,
    hasUnpriced: false,
    costExcludesCache: false,
  };
  return {
    month: '2026-07',
    monthToDate: totals,
    allTime: totals,
    monthlyTokenLimit: null,
    monthlyDollarLimit: null,
    tokenPercent: null,
    dollarPercent: null,
  };
};
const summaryWith = (turns: number) => ({ data: summaryBody(turns) }) as never;

beforeEach(() => {
  // The module remembers the last read for the life of the process; each test starts as the
  // first read in one, unless it says otherwise.
  forgetUsageSnapshot();
  vi.mocked(getUsageSummary).mockReset();
  vi.mocked(getUsageReport).mockReset();
  vi.mocked(getUsageSummary).mockResolvedValue(summaryWith(0));
  vi.mocked(getUsageReport).mockResolvedValue({ data: { rows: [] } } as never);
});

describe('UsageSection', () => {
  it('uses one coherent month-to-date period without competing range controls', async () => {
    const currentTime = new Date();
    const monthStart = Math.floor(
      new Date(currentTime.getFullYear(), currentTime.getMonth(), 1).getTime() / 1000
    );
    render(<UsageSection />);

    await screen.findByTestId('usage-panel');
    expect(screen.queryByRole('group', { name: 'Usage range' })).toBeNull();
    expect(screen.queryByRole('button', { name: '7d' })).toBeNull();
    expect(screen.queryByRole('button', { name: '30d' })).toBeNull();
    expect(screen.queryByRole('button', { name: '90d' })).toBeNull();
    expect(getUsageSummary).toHaveBeenCalledTimes(1);
    expect(getUsageReport).toHaveBeenCalledTimes(2);
    expect(vi.mocked(getUsageReport).mock.calls[0]?.[0]).toMatchObject({
      query: { from: monthStart, group: 'day' },
    });
    expect(vi.mocked(getUsageReport).mock.calls[1]?.[0]).toMatchObject({
      query: { from: monthStart, group: 'model' },
    });
  });

  it('is the deep-link target, with its explanation behind an InfoTip', async () => {
    const { container } = render(<UsageSection />);
    await screen.findByTestId('usage-panel');
    const section = container.querySelector(`#${SETTINGS_SECTION_IDS.usage}`);
    expect(section).not.toBeNull();
    expect(section).toHaveAccessibleName('Usage');
    expect(screen.getByRole('button', { name: 'About Usage' })).toHaveAccessibleDescription(
      'Billed tokens and estimated cost for the current month, grouped by day and model.'
    );
    expect(screen.queryByText(/grouped by day and model/, { selector: 'p' })).toBeNull();
  });

  it('stays one row, with no dollar figure, when costs are hidden', async () => {
    const { rerender } = render(<UsageSection />);
    const panel = await screen.findByTestId('usage-panel');
    expect(panel).toHaveTextContent('$0.00');
    const rowBefore = screen.getByText('This month').closest('.br-setting-row');

    rerender(<UsageSection showCosts={false} />);
    // The same panel and the same row, not a remount: the section does not unmount on the switch.
    expect(screen.getByTestId('usage-panel')).toBe(panel);
    expect(screen.getByText('This month').closest('.br-setting-row')).toBe(rowBefore);
    expect(panel.textContent).not.toContain('$');
    expect(panel).toHaveTextContent('0 tokens · 0 turns');
    // The section's InfoTip stops talking about cost too.
    expect(screen.getByRole('button', { name: 'About Usage' })).toHaveAccessibleDescription(
      copy.sectionHelpNoCost
    );
    expect(copy.sectionHelpNoCost).not.toMatch(/cost|price|\$/i);
  });

  it('loads as one row-sized skeleton', () => {
    vi.mocked(getUsageSummary).mockReturnValue(new Promise(() => {}) as never);
    render(<UsageSection />);
    expect(screen.getByTestId('usage-loading')).toHaveClass('h-10', 'w-full');
  });

  it('keeps a failed fetch visible and offers a working retry', async () => {
    const user = userEvent.setup();
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    vi.mocked(getUsageSummary).mockRejectedValueOnce(new Error('offline'));

    render(<UsageSection />);

    const alert = await screen.findByTestId('usage-load-error');
    expect(alert).toHaveAttribute('role', 'alert');
    expect(alert).toHaveTextContent('Usage data could not be loaded');
    expect(screen.getByRole('heading', { name: 'Usage' })).toBeTruthy();

    await user.click(screen.getByRole('button', { name: 'Retry' }));
    await screen.findByTestId('usage-panel');
    expect(screen.queryByTestId('usage-load-error')).toBeNull();

    consoleError.mockRestore();
  });

  describe('a return to the App tab (vision V4-R7-3)', () => {
    /** A first visit that has finished, then leaving the tab: the next mount is a return. */
    async function visitAndLeave() {
      const first = render(<UsageSection />);
      await screen.findByTestId('usage-panel');
      first.unmount();
    }

    it('draws the row in the first render, with no skeleton, while it asks again', async () => {
      await visitAndLeave();
      vi.mocked(getUsageSummary).mockClear();
      vi.mocked(getUsageSummary).mockReturnValue(new Promise(() => {}) as never);

      render(<UsageSection />);

      // Synchronously after render: nothing was awaited, so this is the first commit.
      expect(screen.getByTestId('usage-panel')).toHaveTextContent('This month');
      expect(screen.queryByTestId('usage-loading')).toBeNull();
      // The daemon is still asked; the seed is a head start, not a cache the row trusts.
      await waitFor(() => expect(getUsageSummary).toHaveBeenCalledTimes(1));
    });

    it('says nothing while the seeded row refreshes: no status line to move what is below', async () => {
      await visitAndLeave();
      vi.mocked(getUsageSummary).mockReturnValue(new Promise(() => {}) as never);

      render(<UsageSection />);

      expect(screen.queryByRole('status')).toBeNull();
      expect(screen.queryByText(copy.refreshing)).toBeNull();
    });

    it('replaces the seed with the daemon’s answer in the same row', async () => {
      await visitAndLeave();
      vi.mocked(getUsageSummary).mockResolvedValue(summaryWith(7));

      render(<UsageSection />);
      const panel = screen.getByTestId('usage-panel');
      expect(panel).toHaveTextContent('0 turns');

      await act(async () => {});
      // Not a remount: the node on screen is the one that was there at the start.
      expect(screen.getByTestId('usage-panel')).toBe(panel);
      expect(panel).toHaveTextContent('7 turns');
    });

    it('keeps the seeded row and names the failure when the refresh cannot be read', async () => {
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => undefined);
      await visitAndLeave();
      vi.mocked(getUsageSummary).mockRejectedValue(new Error('offline'));

      render(<UsageSection />);
      const alert = await screen.findByTestId('usage-load-error');

      expect(alert).toHaveTextContent(copy.loadError);
      expect(alert).toHaveTextContent(copy.loadErrorStale.trim());
      expect(screen.getByTestId('usage-panel')).toBeTruthy();
      consoleError.mockRestore();
    });

    it('still says it is refreshing when the person asked for a retry', async () => {
      const user = userEvent.setup();
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => undefined);
      await visitAndLeave();
      vi.mocked(getUsageSummary).mockRejectedValueOnce(new Error('offline'));

      render(<UsageSection />);
      await screen.findByTestId('usage-load-error');
      vi.mocked(getUsageSummary).mockReturnValue(new Promise(() => {}) as never);
      await user.click(screen.getByRole('button', { name: copy.retry }));

      expect(screen.getByRole('status')).toHaveTextContent(copy.refreshing);
      expect(screen.getByTestId('usage-panel')).toBeTruthy();
      consoleError.mockRestore();
    });

    it('is a skeleton only for the very first read in the process', () => {
      vi.mocked(getUsageSummary).mockReturnValue(new Promise(() => {}) as never);
      render(<UsageSection />);
      expect(screen.getByTestId('usage-loading')).toBeTruthy();
      expect(screen.queryByTestId('usage-panel')).toBeNull();
    });

    it('does not seed a month that is over: the row says "This month"', () => {
      const lastMonth = new Date();
      lastMonth.setDate(1);
      lastMonth.setMonth(lastMonth.getMonth() - 1);
      rememberUsageSnapshot(lastMonth, {
        summary: summaryBody(99),
        dayRows: [],
        modelRows: [],
      });
      vi.mocked(getUsageSummary).mockReturnValue(new Promise(() => {}) as never);

      render(<UsageSection />);

      expect(screen.getByTestId('usage-loading')).toBeTruthy();
      expect(screen.queryByTestId('usage-panel')).toBeNull();
    });

    it('forgets the read when app data is reset, so the old totals are not drawn again', async () => {
      await visitAndLeave();
      vi.mocked(getUsageSummary).mockReturnValue(new Promise(() => {}) as never);

      act(() => {
        window.dispatchEvent(
          new CustomEvent(DATA_RESET_EVENT, { detail: { categories: ['history'] } })
        );
      });
      render(<UsageSection />);

      expect(screen.getByTestId('usage-loading')).toBeTruthy();
      expect(screen.queryByTestId('usage-panel')).toBeNull();
    });

    it('seeds every mount after the first, not only the second', async () => {
      await visitAndLeave();
      for (let visit = 0; visit < 3; visit++) {
        vi.mocked(getUsageSummary).mockReturnValue(new Promise(() => {}) as never);
        const view = render(<UsageSection />);
        expect(screen.getByTestId('usage-panel')).toBeTruthy();
        expect(screen.queryByTestId('usage-loading')).toBeNull();
        view.unmount();
      }
    });
  });
});
