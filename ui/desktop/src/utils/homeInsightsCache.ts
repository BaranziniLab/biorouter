import { userActionHeaders } from './userAction';
import { getSessionActivity, type ActivityWindow } from '../api';

export const HOME_ACTIVITY_DAYS = 155;

// v1 blobs also carried a `recentSessions` list for the Home recent-chats
// section; that section is gone (the sidebar's Recents owns the job), so the
// stray key is simply ignored and dropped on the next persist.
const HOME_INSIGHTS_STORAGE_KEY = 'biorouter-home-insights-v1';

type HomeInsightsCache = {
  activity: ActivityWindow | null;
};

let memoryCache: HomeInsightsCache | undefined;
let inFlightActivity: Promise<ActivityWindow> | null = null;

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

    const parsed = JSON.parse(stored) as Partial<HomeInsightsCache>;
    memoryCache = { activity: parsed.activity ?? null };
  } catch {
    memoryCache = emptyCache;
  }

  return memoryCache;
}

function persistCache(): void {
  if (typeof localStorage === 'undefined') return;

  try {
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
    localStorage.removeItem(HOME_INSIGHTS_STORAGE_KEY);
  }
}
