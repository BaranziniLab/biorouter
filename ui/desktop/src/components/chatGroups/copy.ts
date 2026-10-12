/**
 * The words on the chat tab strip, in one place so the strip and its tests say
 * the same thing (spec 1, copy rules: each area keeps its strings in a
 * `copy.ts`, and tests import them).
 */
export const chatTabStripCopy = {
  /** The tablist's accessible name. */
  tablistLabel: 'Open chats',
  /** The ▾ that lists every tab once some are scrolled out of sight. */
  overflowTrigger: 'Show all chats',
  /** The filter at the top of that menu, shown from {@link TAB_MENU_FILTER_MIN} tabs. */
  filterLabel: 'Filter tabs',
  filterPlaceholder: 'Filter tabs',
  /** The one line the menu shows when the filter matches nothing. */
  noMatch: 'No tabs match',
  close: (title: string) => `Close ${title}`,
  running: 'Running',
  /** The overflow menu's mark for the tab you are on (a check, read as this word). */
  current: 'Current tab',
} as const;

/**
 * The overflow menu grows a filter field once it lists this many tabs (spec
 * 3.16, "searching tabs"). Below it a list is short enough to read at a glance.
 */
export const TAB_MENU_FILTER_MIN = 8;
