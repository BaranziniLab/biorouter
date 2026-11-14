import { isBrowserSurface } from '../../utils/surface';

const SEEN_KEY = 'biorouter:dictation-shared-origin-v1';
const seenHere = new Set<string>();

/** Browser microphone grants apply to an origin, not just this page. */
export function needsSharedOriginNotice(): boolean {
  if (!isBrowserSurface()) return false;
  const origin = window.location.origin;
  if (seenHere.has(origin)) return false;
  try {
    return window.localStorage.getItem(SEEN_KEY) !== origin;
  } catch {
    return true;
  }
}

export function acknowledgeSharedOriginNotice(): void {
  const origin = window.location.origin;
  seenHere.add(origin);
  try {
    window.localStorage.setItem(SEEN_KEY, origin);
  } catch {
    // A blocked store asks again after the page closes.
  }
}

export function resetSharedOriginNoticeForTests(): void {
  seenHere.clear();
  try {
    window.localStorage.removeItem(SEEN_KEY);
  } catch {
    // Tests with blocked storage still clear the in-memory acknowledgement.
  }
}
