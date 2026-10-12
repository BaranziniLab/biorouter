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
 * - INTO the docked summary's column (`column`): as wide as the card, its right edge on the
 *   card's, 8px under it. The rail is the one place on that side of the window that is not the
 *   conversation. The stack used to step 16px LEFT of the card instead, and the final app check
 *   (2026-10-12) found what that means: beside the rail is the transcript, and the sticky
 *   "1 of 2 extensions loaded" stood over the person's own message for minutes ("Toasts do not
 *   cover transcript content"). When the card fills the rail, so the stack cannot fit under it
 *   above the window's bottom inset, the stack keeps its corner height over the card's header,
 *   still in the column: the rail's status line covered for a moment beats the conversation;
 * - beside the summary popover, 16px to its left. The popover is a menu the person opened, it
 *   closes on its own, and it stands above the toast anyway ("toast yields to an open menu" in
 *   main.css).
 *
 * Only what is really in the way counts: the stack's resting box is tested against each surface,
 * so a find bar in the left pane of a split, or a rail beside an open preview, moves nothing. The
 * stack never moves up, so it can never enter the titlebar drag band above it (issue #74). It
 * moves left, and right only to line up with a column it joins (4px: the rail's 12px gutter
 * against the stack's 16px inset). Where stepping left would push the stack off the window, it
 * stays, and the popover stands above it anyway.
 *
 * The move is a `translate` set from two custom properties on the container
 * (`--toast-clear-x`, `--toast-clear-y`; main.css "toast clears the corner"), and a joined
 * column's width a third (`--toast-clear-width`), which App.tsx's container style reads as its
 * min and max width. The move glides over `--dur-fast-max` once the container has been placed,
 * never on its first frame (a toast that arrives while find is open appears below the bar
 * instead of sliding there), never while the window or the sidebar is being resized, and not
 * under reduced motion. The width snaps: it changes by at most the 52px between the stack's 280
 * and the rail's narrowest card.
 *
 * The page's own actions are the one case where the stack stays and the page steps aside. Below
 * about 1600 wide the 760px reading column reaches under the stack's corner, so a page whose first
 * line carries actions at its trailing edge has them under the stack: the final smoke (2026-10-12)
 * found the sticky "1 of 2 extensions loaded" over the Extensions page's load-failure notice, the
 * right 32px of "Copy error" and all of its dismiss mark. There the stack has nowhere to go that
 * covers nothing: under the notice is the list's first switch, left of the actions is the error
 * itself. So a group of such actions says so (`data-toast-aside`), and while the stack stands
 * beside it the watcher writes how far it moves left (`--toast-aside`: out from under the stack
 * and 16px clear of it, but never closer than 8px to the element before it, the title its row
 * names). main.css "toast clears the corner" turns that into a `translate`, so nothing else on the
 * page moves or rewraps. It snaps, and it never goes back from under a pointer resting on it: it
 * returns once the pointer leaves.
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
/**
 * Under a column it joins, the stack keeps this far above the window's bottom edge: the same 16
 * as its inset from the right. Where it cannot, it stays at its corner height, over the column.
 */
export const TOAST_CLEAR_MIN_BOTTOM = 16;

export const TOAST_CLEAR_X = '--toast-clear-x';
export const TOAST_CLEAR_Y = '--toast-clear-y';
/** The width of the column the stack has joined; unset whenever it has joined none. */
export const TOAST_CLEAR_WIDTH = '--toast-clear-width';
/** Set on a container one frame after its first placement; main.css glides only then. */
export const TOAST_PLACED_ATTRIBUTE = 'data-toast-placed';
/** Marks a group of in-page trailing actions that steps left, out from under the stack. */
export const TOAST_ASIDE_ATTRIBUTE = 'data-toast-aside';
/** Written on such a group while the stack stands beside it: how far left it moves, in px. */
export const TOAST_ASIDE = '--toast-aside';
/** A group stepping aside stops this far short of the element before it on its row. */
export const TOAST_ASIDE_SIBLING_GAP = 8;

const CONTAINER = '.Toastify__toast-container';

/**
 * How the stack clears a surface: it steps `below` it, steps left `beside` it, or joins its
 * `column` (as wide as it, right edges aligned, under it when the window has room).
 */
export type ToastClearanceSide = 'below' | 'beside' | 'column';

/** The surfaces a toast must not cover, each owned elsewhere (see the file comment). */
export const TOAST_OBSTACLES: ReadonlyArray<{ selector: string; side: ToastClearanceSide }> = [
  { selector: '.br-find-overlay', side: 'below' },
  { selector: '.br-summary-card', side: 'column' },
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

export interface ToastClearanceLimits {
  /** The stack never moves closer to the window's left edge than this. */
  minLeft?: number;
  /** The window's bottom edge. The stack goes under a joined column only if it ends 16 above. */
  bottom?: number;
}

export interface ToastClearanceMove {
  /** How far left, in px. Negative only to line up with a column it joins. */
  x: number;
  /** How far down, in px, zero or more. */
  y: number;
  /** The joined column's width, in px; absent when the stack has joined none. */
  width?: number;
}

/**
 * Where the stack goes. `stack` is where it sits with no move (its resting corner).
 *
 * A column in the way is joined first. Then each surface still in the way is cleared in turn,
 * and a move can bring another into the way (stepped left of the popover, the stack meets a find
 * bar open beside it), so this repeats until nothing overlaps.
 */
export function toastClearance(
  stack: ClearanceBox,
  obstacles: ReadonlyArray<ToastObstacle>,
  { minLeft = TOAST_CLEAR_MIN_LEFT, bottom = Infinity }: ToastClearanceLimits = {}
): ToastClearanceMove {
  let x = 0;
  let y = 0;
  const height = stack.bottom - stack.top;
  const column = obstacles.find(
    (o) => o.side === 'column' && hasArea(o.box) && overlaps(stack, o.box)
  );
  if (column) {
    x = stack.right - column.box.right;
    const under = column.box.bottom + TOAST_CLEAR_BELOW_GAP;
    if (under + height <= bottom - TOAST_CLEAR_MIN_BOTTOM) y = under - stack.top;
  }
  const joined = column ? column.box.right - column.box.left : undefined;
  const width = joined ?? stack.right - stack.left;
  const usable = obstacles.filter(
    (o) =>
      o.side !== 'column' &&
      hasArea(o.box) &&
      (o.side === 'below' || o.box.left - TOAST_CLEAR_BESIDE_GAP - width >= minLeft)
  );
  for (let pass = 0; pass <= usable.length * 2; pass++) {
    const at = {
      left: stack.right - width - x,
      right: stack.right - x,
      top: stack.top + y,
      bottom: stack.top + height + y,
    };
    const hit = usable.find((o) => overlaps(at, o.box));
    if (!hit) break;
    if (hit.side === 'beside') {
      x = Math.max(x, stack.right - (hit.box.left - TOAST_CLEAR_BESIDE_GAP));
    } else {
      y = Math.max(y, hit.box.bottom + TOAST_CLEAR_BELOW_GAP - stack.top);
    }
  }
  return joined === undefined ? { x, y } : { x, y, width: joined };
}

/**
 * How far a group of in-page actions moves left, from where it rests (`group`), so none of it sits
 * under the stack or within the 16px gap the stack keeps beside a surface. Zero when the stack
 * (where it stands, after its own moves) is not beside the group, or there is no stack. Never so
 * far that it comes within 8px of `before`, the element before it, when that shares its row: a
 * group squeezed against its title stops there, even if the stack still reaches it.
 */
export function toastAside(
  stack: ClearanceBox | null,
  group: ClearanceBox,
  before: ClearanceBox | null = null
): number {
  if (!stack || !hasArea(stack) || !hasArea(group)) return 0;
  if (!overlaps(stack, { ...group, right: group.right + TOAST_CLEAR_BESIDE_GAP })) return 0;
  const clear = group.right - (stack.left - TOAST_CLEAR_BESIDE_GAP);
  const sameRow = before && before.top < group.bottom && group.top < before.bottom;
  const room = sameRow ? group.left - (before.right + TOAST_ASIDE_SIBLING_GAP) : Infinity;
  return Math.max(0, Math.min(clear, room));
}

/**
 * Keep the toast container clear of the corner's surfaces for as long as it exists. Returns the
 * cleanup. App.tsx calls it once, beside the `ToastContainer`.
 *
 * Nothing is watched while there is no toast: an observer on the toast layer notices the
 * container arrive and leave, and only while it is there does a second one follow the page
 * (find opening, the rail docking, the popover opening, a resize, an animation ending, a scroll),
 * each coalesced into one measurement per frame. When it leaves, one last pass gives the page's
 * actions their place back, and so does the cleanup.
 */
export function watchToastClearance(doc: Document = document): () => void {
  const win = doc.defaultView;
  if (!win || typeof MutationObserver === 'undefined' || !doc.body) return () => {};

  let frame = 0;
  let stopped = false;
  let placed: HTMLElement | null = null;
  let page: MutationObserver | null = null;
  const held = new WeakSet<Element>();

  /**
   * Step each `data-toast-aside` group out from under the stack where it stands (`at`), or back
   * when it is gone (`null`). A group under a resting pointer stays where it is until the pointer
   * leaves it, so it never moves out from under a click (checklist 14).
   */
  const stepAside = (at: ClearanceBox | null) => {
    for (const group of doc.querySelectorAll<HTMLElement>(`[${TOAST_ASIDE_ATTRIBUTE}]`)) {
      const now = parseFloat(group.style.getPropertyValue(TOAST_ASIDE)) || 0;
      // Where it rests: its box, less the move it already makes.
      const r = group.getBoundingClientRect();
      const rest = { left: r.left + now, top: r.top, right: r.right + now, bottom: r.bottom };
      const prev = group.previousElementSibling?.getBoundingClientRect();
      const before = prev
        ? { left: prev.left, top: prev.top, right: prev.right, bottom: prev.bottom }
        : null;
      const next = toastAside(at, rest, before);
      if (Math.abs(next - now) < 0.5) continue;
      if (next < now && group.matches(':hover')) {
        if (!held.has(group)) {
          held.add(group);
          group.addEventListener(
            'pointerleave',
            () => {
              held.delete(group);
              schedule();
            },
            { once: true }
          );
        }
        continue;
      }
      if (next > 0) group.style.setProperty(TOAST_ASIDE, `${next}px`);
      else group.style.removeProperty(TOAST_ASIDE);
    }
  };

  const place = () => {
    frame = 0;
    const container = doc.querySelector<HTMLElement>(CONTAINER);
    if (!container) {
      stepAside(null);
      return;
    }
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
    const { x, y, width } = toastClearance(stack, obstacles, {
      bottom: doc.documentElement.clientHeight,
    });
    const nextX = `${x}px`;
    const nextY = `${y}px`;
    const nextWidth = width === undefined ? '' : `${width}px`;
    if (container.style.getPropertyValue(TOAST_CLEAR_X) !== nextX) {
      container.style.setProperty(TOAST_CLEAR_X, nextX);
    }
    if (container.style.getPropertyValue(TOAST_CLEAR_Y) !== nextY) {
      container.style.setProperty(TOAST_CLEAR_Y, nextY);
    }
    // A new width rewraps the stack, so its height changes, and this write is itself a mutation
    // the page observer sees: the next frame measures again at the new width. Joining a column
    // does not depend on that height (only going under it does, at one fixed width), so the
    // width and the height cannot chase each other.
    if (container.style.getPropertyValue(TOAST_CLEAR_WIDTH) !== nextWidth) {
      if (nextWidth) container.style.setProperty(TOAST_CLEAR_WIDTH, nextWidth);
      else container.style.removeProperty(TOAST_CLEAR_WIDTH);
    }
    if (placed !== container) {
      // A new container lands where it belongs; only later moves glide.
      placed = container;
      win.requestAnimationFrame(() => container.setAttribute(TOAST_PLACED_ATTRIBUTE, ''));
    }
    // Where the stack stands once moved (its target, not a frame of the glide), at the width it
    // is taking: the page's actions step aside from that.
    const span = width ?? stack.right - stack.left;
    stepAside({
      left: stack.right - x - span,
      right: stack.right - x,
      top: stack.top + y,
      bottom: stack.bottom + y,
    });
  };

  // Hoisted, as a held group's `pointerleave` above calls it; `view` carries the narrowing a
  // hoisted function does not see on `win`.
  const view: Window = win;
  function schedule() {
    if (!frame && !stopped) frame = view.requestAnimationFrame(place);
  }

  const follow = () => {
    if (page) return;
    page = new MutationObserver(schedule);
    page.observe(doc.body, { subtree: true, childList: true, attributes: true });
    win.addEventListener('resize', schedule);
    doc.addEventListener('animationend', schedule, true);
    doc.addEventListener('transitionend', schedule, true);
    // Actions scrolled into, or out from beside, a standing stack.
    doc.addEventListener('scroll', schedule, true);
  };

  const unfollow = () => {
    if (!page) return;
    page.disconnect();
    page = null;
    win.removeEventListener('resize', schedule);
    doc.removeEventListener('animationend', schedule, true);
    doc.removeEventListener('transitionend', schedule, true);
    doc.removeEventListener('scroll', schedule, true);
  };

  const sync = () => {
    if (doc.querySelector(CONTAINER)) {
      follow();
      schedule();
    } else {
      unfollow();
      // One more pass with no stack, if one stood: the page's actions go back.
      if (placed) schedule();
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
    stopped = true;
    layer.disconnect();
    unfollow();
    if (frame) win.cancelAnimationFrame(frame);
    frame = 0;
    for (const group of doc.querySelectorAll<HTMLElement>(`[${TOAST_ASIDE_ATTRIBUTE}]`)) {
      group.style.removeProperty(TOAST_ASIDE);
    }
  };
}
