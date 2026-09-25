import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type RefObject,
} from 'react';

/**
 * Roving keyboard focus for the sidebar's rows (ui-redesign-spec, Accessibility → Keyboard):
 * ↑/↓ move between rows, Home/End jump to the ends, and Tab leaves the list, so the list is one
 * tab stop however many channels it holds.
 *
 * While focus is inside, the stop follows the row that last had focus. Once focus LEAVES the
 * list, the stop goes back to the preferred row — the current channel — so Tab or Shift+Tab back
 * in always lands where the person is, never on the last row they arrowed past (Q2-46: it came
 * back to "+ Add channel", one Enter away from a duplicate channel).
 *
 * ⚠ **Focus coming back to a control INSIDE a row also takes the stop back (Q3-56).** A team
 * header's + and ⋯ share the header's `tabIndex`, so they are Tab stops only while the header is.
 * Opening the team menu moves focus into a portal — out of the list — so the stop went to the
 * current channel; Escape then put focus back on ⋯, now `tabindex="-1"` beside a `-1` + and
 * header, and Shift+Tab skipped the whole rail to the privacy chip. `onFocus` on the container
 * hands the stop to whichever row the focused element sits in, so the way back is the way in.
 * And while focus is in a menu that a row's control opened, it has not left that row: the stop
 * stays, so Shift+Tab from inside the open team menu — which the menu sends to the stop before
 * its trigger — reaches the header's + too, not the chip. Focus that goes anywhere else, a dialog
 * a menu item opened included, hands the stop back to the current channel.
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
  /** Call from a row's `onFocus`, so the stop follows focus while it stays in the list. */
  onRowFocus(key: string): void;
  onKeyDown(event: KeyboardEvent<HTMLElement>): void;
  /** Put on the container: focus landing anywhere in a row makes that row the stop (Q3-56). */
  onFocus(event: FocusEvent<HTMLElement>): void;
  /** Put on the container: focus leaving the list hands the stop back to the preferred row. */
  onBlur(event: FocusEvent<HTMLElement>): void;
}

/**
 * Whether `element` sits in a menu opened from a control inside `container` — a team's ⋯ menu,
 * or its "Copy for support" submenu. Radix names a menu's trigger in `aria-labelledby`; a submenu
 * names its sub-trigger, which sits in the parent menu, so the chain is followed up to the root.
 */
export function inMenuOpenedFrom(element: Element, container: Element): boolean {
  let content = element.closest('[data-radix-menu-content]');
  for (let depth = 0; content && depth < 4; depth += 1) {
    const labelledBy = content.getAttribute('aria-labelledby');
    const trigger = labelledBy ? element.ownerDocument.getElementById(labelledBy) : null;
    if (!trigger) return false;
    if (container.contains(trigger)) return true;
    content = trigger.closest('[data-radix-menu-content]');
  }
  return false;
}

/** The key of the row `element` sits in (the row itself, or a control in its wrapper), if any. */
export function rowKeyOf(element: Element, container: Element): string | null {
  const item = element.closest(ROW_ITEM_SELECTOR);
  if (!item || !container.contains(item)) return null;
  const row = item.matches(ROW_SELECTOR) ? item : item.querySelector(ROW_SELECTOR);
  return row?.getAttribute('data-crew-row') ?? null;
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

  const onFocus = useCallback((event: FocusEvent<HTMLElement>) => {
    const container = containerRef.current;
    if (!container || !(event.target instanceof Element)) return;
    const key = rowKeyOf(event.target, container);
    if (key) setFocusedKey(key);
  }, []);

  const onBlur = useCallback((event: FocusEvent<HTMLElement>) => {
    const container = containerRef.current;
    const next = event.relatedTarget;
    if (container && next instanceof Node && container.contains(next)) return;
    // Into a menu one of the rows opened: still on that row.
    if (container && next instanceof Element && inMenuOpenedFrom(next, container)) return;
    setFocusedKey(null);
  }, []);

  // Focus that leaves such a menu for anywhere but the list — a dialog its item opened, the
  // composer — has left the row: the stop goes back to the current channel (Q2-46).
  useEffect(() => {
    const onFocusIn = (event: globalThis.FocusEvent) => {
      const container = containerRef.current;
      const target = event.target;
      if (!container || !(target instanceof Element) || container.contains(target)) return;
      if (inMenuOpenedFrom(target, container)) return;
      setFocusedKey(null);
    };
    document.addEventListener('focusin', onFocusIn);
    return () => document.removeEventListener('focusin', onFocusIn);
  }, []);

  return {
    containerRef,
    tabIndexFor: (key) => (key === stop ? 0 : -1),
    onRowFocus: setFocusedKey,
    onKeyDown,
    onFocus,
    onBlur,
  };
}
