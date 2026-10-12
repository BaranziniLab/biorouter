/**
 * A toast steps out of the way of what already sits in its corner (vision V2-R7-4).
 *
 * The toast layer docks in the window's top-right corner (`--toast-inset-top`, App.tsx). Three
 * surfaces open into that same corner, and a toast arriving over them hid their controls:
 *
 * - find in chat (`.br-find-overlay`, conversation/SearchBar.tsx): the count, the info tip, Aa,
 *   both arrows and Close sat under "Chat ID copied" at 1300 wide;
 * - the docked Chat summary (`.br-summary-card`, summary/ChatSummaryRail.tsx): its "To do"
 *   header and progress bar;
 * - the Chat summary popover (`.br-summary-popover`, summary/SummaryHeaderButton.tsx), which
 *   opens in that corner when the rail does not fit.
 *
 * Checklist 15 names exactly this ("a toast covering the find-in-chat input or any band
 * control"). So the stack moves, and the surface stays where the person put it:
 *
 * - below find in chat, 8px under the bar (the bar's own gap under the header band). The bar is
 *   short, so the toast stays in the corner, one step down;
 * - beside the summary, 16px to its left. That is the toast's own inset from the window's edge
 *   and find's from the conversation's, so beside the docked rail the toast sits at the
 *   conversation's corner exactly as it sits at the window's, and with find open there its right
 *   edge lines up with the bar's. The rail runs the height of the window, so below it would be
 *   off the screen.
 *
 * Only what is really in the way counts: the stack's own box is tested against each surface, so
 * a find bar in the left pane of a split, or a rail beside an open preview, moves nothing. The
 * stack only ever moves down or left, so it can never enter the titlebar drag band above it
 * (issue #74). Where stepping left would push the stack off the window, it stays, and the
 * popover stands above it anyway ("toast yields to an open menu" in main.css).
 *
 * The move is a `translate` set from two custom properties on the container
 * (`--toast-clear-x`, `--toast-clear-y`; main.css "toast clears the corner"). It glides over
 * `--dur-fast-max` once the container has been placed, never on its first frame (a toast that
 * arrives while find is open appears below the bar instead of sliding there), never while the
 * window or the sidebar is being resized, and not under reduced motion.
 */

/** The gap under a surface the stack moves below: the find bar's own gap under the band. */
export const TOAST_CLEAR_BELOW_GAP = 8;
/**
 * The gap beside a surface the stack moves left of: 16, the stack's own inset from the window's
 * right edge and find's from the conversation's, so the stack beside the rail lines up with find.
 */
export const TOAST_CLEAR_BESIDE_GAP = 16;
/** The stack never moves closer to the window's left edge than its own inset from the right. */
export const TOAST_CLEAR_MIN_LEFT = 16;

export const TOAST_CLEAR_X = '--toast-clear-x';
export const TOAST_CLEAR_Y = '--toast-clear-y';
/** Set on a container one frame after its first placement; main.css glides only then. */
export const TOAST_PLACED_ATTRIBUTE = 'data-toast-placed';

const CONTAINER = '.Toastify__toast-container';

export type ToastClearanceSide = 'below' | 'beside';

/** The surfaces a toast must not cover, each owned elsewhere (see the file comment). */
export const TOAST_OBSTACLES: ReadonlyArray<{ selector: string; side: ToastClearanceSide }> = [
  { selector: '.br-find-overlay', side: 'below' },
  { selector: '.br-summary-card', side: 'beside' },
  { selector: '.br-summary-popover', side: 'beside' },
];

export interface ClearanceBox {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface ToastObstacle {
  side: ToastClearanceSide;
  box: ClearanceBox;
}

const overlaps = (a: ClearanceBox, b: ClearanceBox) =>
  a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

const hasArea = (b: ClearanceBox) => b.right > b.left && b.bottom > b.top;

/**
 * How far the stack moves left (`x`) and down (`y`), in px, both zero or more.
 *
 * `stack` is where the stack sits with no move (its resting corner). Each surface in the way is
 * cleared in turn, and a move can bring another into the way (stepped left of the rail, the
 * stack meets a find bar open beside it), so this repeats until nothing overlaps.
 */
export function toastClearance(
  stack: ClearanceBox,
  obstacles: ReadonlyArray<ToastObstacle>,
  minLeft: number = TOAST_CLEAR_MIN_LEFT
): { x: number; y: number } {
  const width = stack.right - stack.left;
  const usable = obstacles.filter(
    (o) =>
      hasArea(o.box) &&
      (o.side === 'below' || o.box.left - TOAST_CLEAR_BESIDE_GAP - width >= minLeft)
  );
  let x = 0;
  let y = 0;
  for (let pass = 0; pass <= usable.length * 2; pass++) {
    const at = {
      left: stack.left - x,
      right: stack.right - x,
      top: stack.top + y,
      bottom: stack.bottom + y,
    };
    const hit = usable.find((o) => overlaps(at, o.box));
    if (!hit) break;
    if (hit.side === 'beside') {
      x = Math.max(x, stack.right - (hit.box.left - TOAST_CLEAR_BESIDE_GAP));
    } else {
      y = Math.max(y, hit.box.bottom + TOAST_CLEAR_BELOW_GAP - stack.top);
    }
  }
  return { x, y };
}

/**
 * Keep the toast container clear of the corner's surfaces for as long as it exists. Returns the
 * cleanup. App.tsx calls it once, beside the `ToastContainer`.
 *
 * Nothing is watched while there is no toast: an observer on the toast layer notices the
 * container arrive and leave, and only while it is there does a second one follow the page
 * (find opening, the rail docking, the popover opening, a resize, an animation ending), each
 * coalesced into one measurement per frame.
 */
export function watchToastClearance(doc: Document = document): () => void {
  const win = doc.defaultView;
  if (!win || typeof MutationObserver === 'undefined' || !doc.body) return () => {};

  let frame = 0;
  let placed: HTMLElement | null = null;
  let page: MutationObserver | null = null;

  const place = () => {
    frame = 0;
    const container = doc.querySelector<HTMLElement>(CONTAINER);
    if (!container) return;
    const style = win.getComputedStyle(container);
    const right = doc.documentElement.clientWidth - (parseFloat(style.right) || 0);
    const top = parseFloat(style.top) || 0;
    const stack = {
      left: right - container.offsetWidth,
      right,
      top,
      bottom: top + container.offsetHeight,
    };
    const obstacles = TOAST_OBSTACLES.flatMap(({ selector, side }) =>
      Array.from(doc.querySelectorAll(selector), (el) => {
        const r = el.getBoundingClientRect();
        return { side, box: { left: r.left, top: r.top, right: r.right, bottom: r.bottom } };
      })
    );
    const { x, y } = toastClearance(stack, obstacles);
    const nextX = `${x}px`;
    const nextY = `${y}px`;
    if (container.style.getPropertyValue(TOAST_CLEAR_X) !== nextX) {
      container.style.setProperty(TOAST_CLEAR_X, nextX);
    }
    if (container.style.getPropertyValue(TOAST_CLEAR_Y) !== nextY) {
      container.style.setProperty(TOAST_CLEAR_Y, nextY);
    }
    if (placed !== container) {
      // A new container lands where it belongs; only later moves glide.
      placed = container;
      win.requestAnimationFrame(() => container.setAttribute(TOAST_PLACED_ATTRIBUTE, ''));
    }
  };

  const schedule = () => {
    if (!frame) frame = win.requestAnimationFrame(place);
  };

  const follow = () => {
    if (page) return;
    page = new MutationObserver(schedule);
    page.observe(doc.body, { subtree: true, childList: true, attributes: true });
    win.addEventListener('resize', schedule);
    doc.addEventListener('animationend', schedule, true);
    doc.addEventListener('transitionend', schedule, true);
  };

  const unfollow = () => {
    if (!page) return;
    page.disconnect();
    page = null;
    win.removeEventListener('resize', schedule);
    doc.removeEventListener('animationend', schedule, true);
    doc.removeEventListener('transitionend', schedule, true);
  };

  const sync = () => {
    if (doc.querySelector(CONTAINER)) {
      follow();
      schedule();
    } else {
      unfollow();
      placed = null;
    }
  };

  // react-toastify's `section.Toastify` holds one container per corner, and only while a toast
  // is showing there. Without it (a test, an error screen), watch the body's direct children.
  const layer = new MutationObserver(sync);
  layer.observe(doc.querySelector('section.Toastify') ?? doc.body, {
    childList: true,
    subtree: false,
  });
  sync();

  return () => {
    layer.disconnect();
    unfollow();
    if (frame) win.cancelAnimationFrame(frame);
    frame = 0;
  };
}
