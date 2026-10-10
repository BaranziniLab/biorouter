import { userActionHeaders } from './userAction';
import { getSessionActivity, type ActivityWindow } from '../api';

/**
 * 53 weeks: the daemon's maximum (`get_activity` clamps to 371), so the usage
 * card's `1y` range (365 days) is always inside the window and the heatmap
 * draws one column per week of it. Owner decision 6.4: no backend change.
 */
export const HOME_ACTIVITY_DAYS = 371;

/**
 * v2 since the window grew from 155 to 371 days. A v1 blob holds a 155-day
 * window that would understate every `1y` figure, so it is never read: it is
 * removed on the next persist, and Home shows its loading state once.
 */
const HOME_INSIGHTS_STORAGE_KEY = 'biorouter-home-insights-v2';
const RETIRED_STORAGE_KEYS = ['biorouter-home-insights-v1'];

type HomeInsightsCache = {
  activity: ActivityWindow | null;
};

let memoryCache: HomeInsightsCache | undefined;
let inFlightActivity: Promise<ActivityWindow> | null = null;

/** A blob from another build, or a corrupt one, must not crash Home. */
function isActivityWindow(value: unknown): value is ActivityWindow {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Partial<ActivityWindow>;
  return (
    typeof candidate.start === 'string' &&
    typeof candidate.end === 'string' &&
    Array.isArray(candidate.days)
  );
}

function readCache(): HomeInsightsCache {
  if (memoryCache) return memoryCache;

  const emptyCache = { activity: null };
  if (typeof localStorage === 'undefined') {
    memoryCache = emptyCache;
    return memoryCache;
  }

  try {
    const stored = localStorage.getItem(HOME_INSIGHTS_STORAGE_KEY);
    if (!stored) {
      memoryCache = emptyCache;
      return memoryCache;
    }

    const parsed = JSON.parse(stored) as Partial<HomeInsightsCache> | null;
    const activity = parsed?.activity;
    memoryCache = { activity: isActivityWindow(activity) ? activity : null };
  } catch {
    memoryCache = emptyCache;
  }

  return memoryCache;
}

function persistCache(): void {
  if (typeof localStorage === 'undefined') return;

  try {
    for (const key of RETIRED_STORAGE_KEYS) localStorage.removeItem(key);
    localStorage.setItem(HOME_INSIGHTS_STORAGE_KEY, JSON.stringify(readCache()));
  } catch {
    // Rendering from the in-memory cache still provides the warm-navigation path.
  }
}

export function getCachedHomeActivity(): ActivityWindow | null {
  return readCache().activity;
}

export function cacheHomeActivity(activity: ActivityWindow): void {
  readCache().activity = activity;
  persistCache();
}

export async function refreshHomeActivity(): Promise<ActivityWindow> {
  if (inFlightActivity) return inFlightActivity;

  inFlightActivity = userActionHeaders()
    .then((headers) =>
      getSessionActivity<true>({
        headers,
        query: { days: HOME_ACTIVITY_DAYS },
        throwOnError: true,
      })
    )
    .then((response) => {
      cacheHomeActivity(response.data);
      return response.data;
    })
    .finally(() => {
      inFlightActivity = null;
    });

  return inFlightActivity;
}

export function preloadHomeActivity(): void {
  if (getCachedHomeActivity() !== null) return;
  void refreshHomeActivity().catch(() => undefined);
}

export function clearHomeInsightsCache(): void {
  memoryCache = undefined;
  inFlightActivity = null;
  if (typeof localStorage !== 'undefined') {
    try {
      localStorage.removeItem(HOME_INSIGHTS_STORAGE_KEY);
      for (const key of RETIRED_STORAGE_KEYS) localStorage.removeItem(key);
    } catch {
      // Storage can be blocked; the in-memory cache is already cleared.
    }
  }
}
