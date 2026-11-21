import { CHAT_MEASURE } from '../Layout/yieldLadder';

/**
 * Whether Home's usage card has room, and in which form (owner, 2026-11-19).
 *
 * The card shares Home's column with the greeting, above the composer. It is the one thing
 * there that may yield, and it yields before it can touch anything:
 *
 *   'full'    — the tiles, the heat map and its footnote;
 *   'compact' — the tiles only: the heat map and its footnote fold away first, when the card
 *               would otherwise not leave the gap above the composer;
 *   'hidden'  — no card: the greeting and the composer.
 *
 * WIDTH. The card shows only at its whole measure. Its heat map is the one thing on Home whose
 * size is computed (one fluid column per week), so a narrower card is a smaller grid, not a
 * shorter line. The window's old floor (288 + 760 = 1048) existed to keep the grid at that
 * measure; the window may now narrow past it so that the composer can shrink, and the card
 * leaves instead of shrinking with it.
 *
 * HEIGHT. The card's natural height, with and without the heat map, is measured while it is
 * folded away too (a folded box is clipped, not unmounted), so every input is independent of
 * the answer and there is no loop: the room above the composer and the greeting's height do
 * not depend on the card. The gap is the column's own 24px bottom padding
 * (`.biorouter-home-column` in `main.css`), counted because the column is measured with it, so
 * a card that fits always leaves at least that much ground above the composer.
 *
 * An unmeasured layout (a first paint, jsdom) keeps the card whole: it never flashes away.
 */
export type HomeUsageFit = 'full' | 'compact' | 'hidden';

/** The card's measure: the chat column's, which is the composer card's widest. */
export const HOME_USAGE_MIN_WIDTH = CHAT_MEASURE;

/**
 * The ground the card always leaves above the composer: the column's bottom padding. Pinned
 * to `main.css` by `homeUsageFit.test.ts`, and measured in `homeLayout.browser.test.tsx`.
 */
export const HOME_USAGE_MIN_GAP = 24;

/** Sub-pixel slack, so a fractional box on the boundary is not read as a miss. */
const SLACK = 0.5;

export interface HomeUsageMeasure {
  /** The column's width: the chat measure when Home has room for it. */
  columnWidth: number;
  /** The scroller's height: everything above the composer. */
  roomHeight: number;
  /** The column without the card: the greeting and the column's padding (the gap included). */
  restHeight: number;
  /** The card's natural height with its heat map and footnote. */
  fullHeight: number;
  /** The card's natural height without them. */
  compactHeight: number;
}

function measured(value: number): boolean {
  return Number.isFinite(value) && value > 0;
}

export function homeUsageFit(m: HomeUsageMeasure | null): HomeUsageFit {
  if (!m) return 'full';
  if (!measured(m.columnWidth) || !measured(m.roomHeight) || !measured(m.fullHeight)) {
    return 'full';
  }
  if (m.columnWidth < HOME_USAGE_MIN_WIDTH - SLACK) return 'hidden';
  const rest = Number.isFinite(m.restHeight) ? Math.max(0, m.restHeight) : 0;
  if (rest + m.fullHeight <= m.roomHeight + SLACK) return 'full';
  if (measured(m.compactHeight) && rest + m.compactHeight <= m.roomHeight + SLACK) {
    return 'compact';
  }
  return 'hidden';
}
