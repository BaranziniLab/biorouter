/**
 * The left sidebar's width: its bounds, its clamp, and the stored preference.
 *
 * WHY THIS IS A SEPARATE MODULE. The sidebar was a fixed `15rem` written once
 * inside a React component, and the number leaked: `main.ts` derived the OS
 * window's `minWidth` from it (until 2026-11-19; the floor is now the ladder's
 * `WINDOW_MIN_WIDTH`), and `styles/measures.test.ts` re-parsed it out of the
 * component's source. Now that the width is a range rather than a value,
 * several files reason about the same three numbers, and a copy that drifted
 * would let a docked sidebar squeeze the 760px reading column — a layout bug
 * jsdom cannot see, because it computes no layout.
 *
 * So the numbers and the arithmetic live here, free of React and the DOM, the
 * way `useArtifactPanel`'s clamp and `utils/messageClamp.ts`'s thresholds do.
 * Everything below is a pure function of its arguments (the storage helpers take
 * their storage), which is what makes the bounds provable in a unit test rather
 * than only observable by dragging the real app.
 */

/**
 * The floor. 10% under the 240px the sidebar shipped at — narrow enough to buy
 * back a meaningful slice of the canvas, wide enough that a chat title is still
 * a title rather than a truncation. (A comment here once said the OS window's
 * `minWidth` was derived from it; it never was, and is now the yield ladder's
 * `WINDOW_MIN_WIDTH`, below which the sidebar has long since folded.)
 */
export const SIDEBAR_MIN_WIDTH = 216;

/**
 * The default. 20% wider than the 240px that shipped, which is the point of the
 * change: at 240 a conversation title ran out of room mid-phrase, and the list
 * read as a column of prefixes.
 */
export const SIDEBAR_DEFAULT_WIDTH = 288;

/**
 * The ceiling — 25% over the default.
 *
 * Not an arbitrary stop. `SIDEBAR_MAX_WIDTH + 760` (the chat measure) is exactly
 * `SIDEBAR_COMPACT_WIDTH`, rung 1 of the yield ladder: at the narrowest window
 * that still gives the sidebar a column of its own, even a fully widened sidebar
 * leaves the reading column its whole measure. Below that width rung 1 has
 * already collapsed the sidebar to an overlay, where it costs the chat nothing.
 * So no sidebar width inside these bounds can starve the transcript.
 */
export const SIDEBAR_MAX_WIDTH = 360;

/** Per-viewer convenience, so `localStorage` is the right home for it. */
export const SIDEBAR_WIDTH_STORAGE_KEY = 'biorouter:sidebar-width';

/**
 * How far one arrow key moves the drag handle. Small enough to place the edge
 * precisely, large enough that crossing the whole 144px range is not a chore.
 */
export const SIDEBAR_WIDTH_KEYBOARD_STEP = 8;

/**
 * Bring any number into the sidebar's bounds.
 *
 * Total on purpose: a non-finite input resolves to the default rather than
 * propagating `NaN` into a CSS variable, where it would silently collapse the
 * sidebar to zero and look like a rendering bug rather than a bad read.
 */
export function clampSidebarWidth(value: number): number {
  if (!Number.isFinite(value)) return SIDEBAR_DEFAULT_WIDTH;
  return Math.min(Math.max(Math.round(value), SIDEBAR_MIN_WIDTH), SIDEBAR_MAX_WIDTH);
}

type WidthStorage = Pick<Storage, 'getItem' | 'setItem'>;

function defaultStorage(): WidthStorage | null {
  try {
    return typeof window === 'undefined' ? null : window.localStorage;
  } catch {
    // Accessing localStorage itself throws when site data is blocked.
    return null;
  }
}

/**
 * The user's stored width, clamped.
 *
 * ⚠ The clamp on READ is the load-bearing half. A width stored by an earlier
 * build — when the sidebar was a flat 240, or if the bounds move again — must
 * never place today's sidebar outside today's range. Reading is also the one
 * place a stored value can be garbage (hand-edited, half-written, from a
 * different key namespace), so an unparseable value resolves to the default
 * instead of failing.
 */
export function readStoredSidebarWidth(storage: WidthStorage | null = defaultStorage()): number {
  if (!storage) return SIDEBAR_DEFAULT_WIDTH;
  try {
    const stored = storage.getItem(SIDEBAR_WIDTH_STORAGE_KEY);
    if (stored === null) return SIDEBAR_DEFAULT_WIDTH;
    return clampSidebarWidth(Number.parseFloat(stored));
  } catch {
    return SIDEBAR_DEFAULT_WIDTH;
  }
}

/**
 * Persist a width, clamped on the way in as well.
 *
 * Writes are swallowed rather than thrown: a private window or a browser with
 * site data switched off must still let the user resize the sidebar for this
 * session. Losing the preference is a smaller failure than losing the drag.
 */
export function writeStoredSidebarWidth(
  width: number,
  storage: WidthStorage | null = defaultStorage()
): void {
  if (!storage) return;
  try {
    storage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(clampSidebarWidth(width)));
  } catch {
    // Storage is full or unavailable; the in-memory width still stands.
  }
}
