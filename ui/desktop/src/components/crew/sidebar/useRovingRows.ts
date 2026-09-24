import { useCallback, useRef, useState, type KeyboardEvent, type RefObject } from 'react';

/**
 * Roving keyboard focus for the sidebar's rows (ui-redesign-spec, Accessibility → Keyboard):
 * ↑/↓ move between rows, Home/End jump to the ends, and Tab leaves the list from whichever row
 * last had focus, so the list is one tab stop however many channels it holds.
 *
 * A row is an element carrying `data-crew-row={key}`, wrapped in (or equal to) an element
 * carrying `data-crew-row-item`, so a keypress on a control INSIDE a row's wrapper — a team
 * header's + or ⋯ — still moves from that row. Rows are read from the DOM in document order at
 * keypress time, so a collapsed section's rows (unmounted) are skipped without bookkeeping.
 */
export const ROW_SELECTOR = '[data-crew-row]';
export const ROW_ITEM_SELECTOR = '[data-crew-row-item]';

export interface RovingRows {
  containerRef: RefObject<HTMLDivElement | null>;
  /** `0` for the row that holds the list's tab stop, `-1` for the rest. */
  tabIndexFor(key: string): 0 | -1;
  /** Call from a row's `onFocus`, so Tab returns to the row the person last used. */
  onRowFocus(key: string): void;
  onKeyDown(event: KeyboardEvent<HTMLElement>): void;
}

/**
 * @param keys every row key currently rendered, in order.
 * @param preferredKey the row that holds the tab stop when none has had focus (the active channel).
 */
export function useRovingRows(keys: readonly string[], preferredKey: string | null): RovingRows {
  const containerRef = useRef<HTMLDivElement>(null);
  const [focusedKey, setFocusedKey] = useState<string | null>(null);

  const stop =
    focusedKey && keys.includes(focusedKey)
      ? focusedKey
      : preferredKey && keys.includes(preferredKey)
        ? preferredKey
        : (keys[0] ?? null);

  const onKeyDown = useCallback((event: KeyboardEvent<HTMLElement>) => {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const container = containerRef.current;
    const target = event.target instanceof HTMLElement ? event.target : null;
    if (!container || !target) return;
    const item = target.closest(ROW_ITEM_SELECTOR);
    if (!item || !container.contains(item)) return;
    // A disabled row cannot take focus, so it is not a stop.
    const rows = Array.from(container.querySelectorAll<HTMLElement>(ROW_SELECTOR)).filter(
      (row) => !row.matches(':disabled')
    );
    if (rows.length === 0) return;
    const current = rows.findIndex((row) => item === row || item.contains(row));
    if (current < 0) return;
    const next =
      event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? rows.length - 1
          : event.key === 'ArrowDown'
            ? Math.min(rows.length - 1, current + 1)
            : Math.max(0, current - 1);
    event.preventDefault();
    rows[next]?.focus();
  }, []);

  return {
    containerRef,
    tabIndexFor: (key) => (key === stop ? 0 : -1),
    onRowFocus: setFocusedKey,
    onKeyDown,
  };
}
