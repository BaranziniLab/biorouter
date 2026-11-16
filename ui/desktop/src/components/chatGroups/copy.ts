import type { PaneShortfall } from '../Layout/yieldLadder';

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

/**
 * The words on a compact pane (rung 5 of the yield ladder): a pane in a split too small for its
 * chat shows its glyph, its title, one state line and one action in place of the chat. Sentence
 * case, no dashes, no exclamation marks. "Listening…" is `DICTATION_LABELS.listening`, imported
 * where it is drawn rather than restated here.
 */
export const compactPaneCopy = {
  shortfall: {
    width: 'Too narrow to show',
    height: 'Too short to show',
    both: 'Too small to show',
  },
  running: chatTabStripCopy.running, // 'Running'
  /** PhaseGateDialog's word for a chat that needs the person. */
  waiting: 'Waiting for you',
  grow: { width: 'Make wider', height: 'Make taller', both: 'Make larger' },
  merge: 'Merge panes',
  /** The action's accessible name: the visible verb, then the visible title. */
  actionName: (label: string, title: string) => `${label}, ${title}`,
  help: (shortfall: PaneShortfall, canGrow: boolean) => {
    const bigger = { width: 'wider', height: 'taller', both: 'larger' }[shortfall];
    return canGrow
      ? `The chat is hidden, not closed. It comes back as it was when the pane is ${bigger}.`
      : 'The chat is hidden, not closed. Make the window larger, or merge the panes into one.';
  },
  announceWaiting: (title: string) => `${title} is waiting for you`,
} as const;
