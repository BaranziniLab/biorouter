import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { getUsageReport, getUsageSummary } from '../../../api';
import { loadUsage, warmUsageSnapshot } from './loadUsage';
import UsageSection from './UsageSection';
import { forgetUsageSnapshot, readUsageSnapshot } from './usageSnapshot';

vi.mock('../../../api', () => ({
  getUsageReport: vi.fn(),
  getUsageSummary: vi.fn(),
}));

const totals = {
  inputTokens: 0,
  outputTokens: 0,
  totalTokens: 0,
  cacheReadTokens: 0,
  cacheCreationTokens: 0,
  turns: 4,
  cost: 0,
  hasUnpriced: false,
  costExcludesCache: false,
};
const summary = {
  month: '2026-07',
  monthToDate: totals,
  allTime: totals,
  monthlyTokenLimit: null,
  monthlyDollarLimit: null,
  tokenPercent: null,
  dollarPercent: null,
};

beforeEach(() => {
  forgetUsageSnapshot();
  vi.mocked(getUsageSummary).mockReset();
  vi.mocked(getUsageReport).mockReset();
  vi.mocked(getUsageSummary).mockResolvedValue({ data: summary } as never);
  vi.mocked(getUsageReport).mockResolvedValue({ data: { rows: [] } } as never);
});

describe('loadUsage', () => {
  it('reads the month to date once per payload and remembers it for the next mount', async () => {
    const monthStart = Math.floor(
      new Date(new Date().getFullYear(), new Date().getMonth(), 1).getTime() / 1000
    );

    const read = await loadUsage();

    expect(read.summary).toBe(summary);
    expect(getUsageSummary).toHaveBeenCalledTimes(1);
    expect(getUsageReport).toHaveBeenCalledTimes(2);
    expect(vi.mocked(getUsageReport).mock.calls.map(([options]) => options?.query)).toEqual([
      expect.objectContaining({ from: monthStart, group: 'day' }),
      expect.objectContaining({ from: monthStart, group: 'model' }),
    ]);
    expect(readUsageSnapshot()?.summary).toBe(summary);
  });

  it('remembers nothing when a read fails, and says so to the caller', async () => {
    vi.mocked(getUsageSummary).mockRejectedValue(new Error('offline'));
    await expect(loadUsage()).rejects.toThrow('offline');
    expect(readUsageSnapshot()).toBeNull();
  });
});

describe('warmUsageSnapshot (asked while Settings opens)', () => {
  it('reads this month once when nothing has, so the first App visit is at rest too', async () => {
    warmUsageSnapshot();
    await waitFor(() => expect(readUsageSnapshot()).not.toBeNull());
    expect(getUsageSummary).toHaveBeenCalledTimes(1);

    vi.mocked(getUsageSummary).mockReturnValue(new Promise(() => {}) as never);
    render(<UsageSection />);
    // Nothing awaited: the first commit of the first visit already has the row.
    expect(screen.getByTestId('usage-panel')).toHaveTextContent('4 turns');
    expect(screen.queryByTestId('usage-loading')).toBeNull();
  });

  it('asks nothing when this month has already been read', async () => {
    await loadUsage();
    vi.mocked(getUsageSummary).mockClear();
    warmUsageSnapshot();
    await Promise.resolve();
    expect(getUsageSummary).not.toHaveBeenCalled();
  });

  it('is silent when the read fails: the tab reads again and says what it always said', async () => {
    vi.mocked(getUsageSummary).mockRejectedValue(new Error('offline'));
    const unhandled = vi.fn();
    process.on('unhandledRejection', unhandled);
    try {
      warmUsageSnapshot();
      await waitFor(() => expect(getUsageSummary).toHaveBeenCalledTimes(1));
      await new Promise((done) => setTimeout(done, 20));
      expect(unhandled).not.toHaveBeenCalled();
      expect(readUsageSnapshot()).toBeNull();
    } finally {
      process.off('unhandledRejection', unhandled);
    }
  });
});
