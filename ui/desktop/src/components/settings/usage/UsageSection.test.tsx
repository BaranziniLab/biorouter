import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { getUsageReport, getUsageSummary } from '../../../api';
import { settingsUsageCopy as copy } from '../../usage/copy';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import UsageSection from './UsageSection';

vi.mock('../../../api', () => ({
  getUsageReport: vi.fn(),
  getUsageSummary: vi.fn(),
}));

beforeEach(() => {
  vi.mocked(getUsageSummary).mockReset();
  vi.mocked(getUsageReport).mockReset();
  vi.mocked(getUsageSummary).mockResolvedValue({
    data: {
      month: '2026-07',
      monthToDate: {
        inputTokens: 0,
        outputTokens: 0,
        totalTokens: 0,
        cacheReadTokens: 0,
        cacheCreationTokens: 0,
        turns: 0,
        cost: 0,
        hasUnpriced: false,
        costExcludesCache: false,
      },
      allTime: {
        inputTokens: 0,
        outputTokens: 0,
        totalTokens: 0,
        cacheReadTokens: 0,
        cacheCreationTokens: 0,
        turns: 0,
        cost: 0,
        hasUnpriced: false,
        costExcludesCache: false,
      },
      monthlyTokenLimit: null,
      monthlyDollarLimit: null,
      tokenPercent: null,
      dollarPercent: null,
    },
  } as never);
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
});
