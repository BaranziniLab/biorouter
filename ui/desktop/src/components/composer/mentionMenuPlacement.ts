/**
 * Where the Commands and @ lists open (spec 3.7, V2-R5-2).
 *
 * The list opens upward from the composer CARD's top edge, 8px clear of it,
 * starting at the text's left edge. It used to hang from the text field's top,
 * which sits 11px inside the card (its 1px border and 10px of padding, and
 * further down under a queue or a chips row), so the list's bottom 3px covered
 * the card's coral focus edge, and in a card holding chips it covered the chips.
 *
 * Plain geometry with no imports, so the Chromium test
 * (`composerGeometry.browser.test.ts`) bundles this module into its page and
 * lays a list out with the same code `ChatInput` and `MentionPopover` run.
 */

/** Every composer card carries this class (`composerCard.ts`). */
export const COMPOSER_CARD_SELECTOR = '.biorouter-composer-card';

/** Between the list's bottom edge and the card's top edge. */
export const MENTION_MENU_GAP = 8;

/** The list at its tallest (it was the `max-h-72` utility). */
export const MENTION_MENU_MAX_HEIGHT = 288;

/**
 * The list's top never rises above this line. The top 44px of a chat is a
 * window-drag band that eats clicks on whatever overlaps it (CLAUDE.md,
 * "Toasts"); 8px below it is the margin the Tools popover keeps too
 * (`TOOLS_POPOVER_COLLISION_PADDING`). A card high in the window gets a shorter
 * list that scrolls, rather than one that runs under the band or off the window.
 */
export const MENTION_MENU_TOP_CLEARANCE = 52;

/** One 32px row and the list's 4px padding above and below. */
export const MENTION_MENU_MIN_HEIGHT = 40;

const MENU_MIN_WIDTH = 320;
const MENU_MAX_WIDTH = 448;
const WINDOW_MARGIN = 8;

/** The point the list opens from: the text's left edge, the card's top edge. */
export interface MentionMenuAnchor {
  x: number;
  y: number;
}

export function mentionMenuAnchor(field: Element): MentionMenuAnchor {
  const card = field.closest(COMPOSER_CARD_SELECTOR) ?? field;
  return { x: field.getBoundingClientRect().left, y: card.getBoundingClientRect().top };
}

/** The list's fixed-position box, in px. `top` is its bottom edge before the lift. */
export interface MentionMenuPlacement {
  left: number;
  top: number;
  width: number;
  maxHeight: number;
  transform: string;
}

export function mentionMenuPlacement(
  anchor: MentionMenuAnchor,
  windowWidth: number
): MentionMenuPlacement {
  const width = Math.max(MENU_MIN_WIDTH, Math.min(MENU_MAX_WIDTH, windowWidth - 2 * WINDOW_MARGIN));
  const left = Math.min(
    Math.max(WINDOW_MARGIN, anchor.x),
    Math.max(WINDOW_MARGIN, windowWidth - width - WINDOW_MARGIN)
  );
  const bottom = anchor.y - MENTION_MENU_GAP;
  return {
    left,
    top: bottom,
    width,
    maxHeight: Math.max(
      MENTION_MENU_MIN_HEIGHT,
      Math.min(MENTION_MENU_MAX_HEIGHT, bottom - MENTION_MENU_TOP_CLEARANCE)
    ),
    // Lifted by its own height, so it grows upward from `top`.
    transform: 'translateY(-100%)',
  };
}
