import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ActivityWindow } from '../api';

const mocks = vi.hoisted(() => ({
  getSessionActivity: vi.fn(),
}));

vi.mock('../api', () => ({
  getSessionActivity: mocks.getSessionActivity,
}));

const activity: ActivityWindow = {
  start: '2026-02-11',
  end: '2026-07-15',
  maxSessions: 4,
  maxTokens: 1000,
  tokensComplete: true,
  currentStreak: 3,
  longestStreak: 8,
  days: [],
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.resetModules();
  localStorage.clear();
});

describe('homeInsightsCache', () => {
  it('restores activity after a module reload', async () => {
    const cache = await import('./homeInsightsCache');
    cache.cacheHomeActivity(activity);

    vi.resetModules();
    const restoredCache = await import('./homeInsightsCache');

    expect(restoredCache.getCachedHomeActivity()).toEqual(activity);
  });

  it('never reads a v1 blob, whose 155-day window would understate the year', async () => {
    // v1 held a 155-day window (and, before that, a retired `recentSessions`
    // list). Reading it would show `1y` figures for five months.
    localStorage.setItem(
      'biorouter-home-insights-v1',
      JSON.stringify({ activity, recentSessions: [{ id: 'stale' }] })
    );
    const cache = await import('./homeInsightsCache');
    expect(cache.getCachedHomeActivity()).toBeNull();

    // The next persist drops it.
    cache.cacheHomeActivity(activity);
    expect(localStorage.getItem('biorouter-home-insights-v1')).toBeNull();
    expect(localStorage.getItem('biorouter-home-insights-v2')).not.toBeNull();
  });

  it('survives a corrupt or foreign blob without crashing', async () => {
    localStorage.setItem('biorouter-home-insights-v2', JSON.stringify({ activity: { days: 3 } }));
    const cache = await import('./homeInsightsCache');
    expect(cache.getCachedHomeActivity()).toBeNull();

    vi.resetModules();
    localStorage.setItem('biorouter-home-insights-v2', 'not json');
    const again = await import('./homeInsightsCache');
    expect(again.getCachedHomeActivity()).toBeNull();
  });

  it('asks the daemon for its full 371-day window', async () => {
    mocks.getSessionActivity.mockResolvedValue({ data: activity });
    const cache = await import('./homeInsightsCache');
    expect(cache.HOME_ACTIVITY_DAYS).toBe(371);
    await cache.refreshHomeActivity();
    expect(mocks.getSessionActivity).toHaveBeenCalledWith(
      expect.objectContaining({ query: { days: 371 } })
    );
  });

  it('deduplicates overlapping activity prefetch and view requests', async () => {
    let finishRequest: ((value: { data: ActivityWindow }) => void) | undefined;
    mocks.getSessionActivity.mockReturnValue(
      new Promise((resolve) => {
        finishRequest = resolve;
      })
    );
    const cache = await import('./homeInsightsCache');

    cache.preloadHomeActivity();
    const viewLoad = cache.refreshHomeActivity();

    await vi.waitFor(() => expect(mocks.getSessionActivity).toHaveBeenCalledTimes(1));
    finishRequest?.({ data: activity });
    await viewLoad;
    expect(cache.getCachedHomeActivity()).toEqual(activity);
  });

  it('does not prefetch again when persisted activity is available', async () => {
    const cache = await import('./homeInsightsCache');
    cache.cacheHomeActivity(activity);

    cache.preloadHomeActivity();

    expect(mocks.getSessionActivity).not.toHaveBeenCalled();
  });
});
