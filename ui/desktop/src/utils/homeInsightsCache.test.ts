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

  it('ignores the retired recentSessions key in a persisted v1 blob', async () => {
    // Blobs written before the Home recent-chats section was removed carry an
    // extra key; reading them must neither crash nor resurrect it.
    localStorage.setItem(
      'biorouter-home-insights-v1',
      JSON.stringify({ activity, recentSessions: [{ id: 'stale' }] })
    );
    const cache = await import('./homeInsightsCache');
    expect(cache.getCachedHomeActivity()).toEqual(activity);
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
