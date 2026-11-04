import * as React from 'react';
import { isFocusFromTabKey } from '../ui/Tooltip';

/**
 * When the folder chip's card opens and closes (`FolderAccessCard`), and where it hangs.
 *
 * The rules are InfoTip's (`ui/info-tip.tsx`), at a hover card's pace, with the differences that
 * come from what the card holds (Revoke buttons and, in a browser, a key field) and from where it
 * hangs: ABOVE the whole composer card, not on the chip, so a card a hover opened never covers the
 * draft, `+` or Tools (`folderCardAnchorRect`). So
 *   - mouse or pen hover opens it after {@link FOLDER_CARD_OPEN_DELAY_MS}. Leaving the chip or the
 *     card closes it after {@link FOLDER_CARD_CLOSE_GRACE_MS}; a pointer still MOVING along its
 *     way to the other one (`pointerGraceArea`) re-arms that grace, because the way up from the
 *     chip crosses the composer. Entering either one cancels it, and a pointer that stops on the
 *     way, or strays off it, lets it run out;
 *   - a Tab onto the chip opens it at once (Tooltip's gate, so a focus a program restores never
 *     does, including the one `close` makes); the chip's blur closes it unless focus went into
 *     the card;
 *   - Escape and a press outside close it (a press on the chip is not outside). Focus goes back
 *     to the chip only for a close the person made from the keyboard or the chip: Escape,
 *     Shift+Tab off the card's own surface, the chip's click. An outside press or focus leaves
 *     focus where that interaction puts it, as Radix's own non-modal popover does, so the press
 *     that closed the card never hands focus to the chip mid-click;
 *   - a PRESS inside the card pins it, so clicking Revoke or typing the key never loses the card
 *     to a pointer that wandered off; a pinned card ignores pointer leave and scroll;
 *   - `togglePin` is the locked chip's click (and a tap, since touch never hovers); a keyboard
 *     activation also moves focus into the card, so Revoke can be reached;
 *   - while `suppressed` (the native folder chooser is open) it is shut, and nothing a hover or
 *     a focus armed during that time opens it once the chooser closes (#50).
 *
 * InfoTip itself is untouched: it shares the approach, not the code, so its behaviour cannot move.
 */

export const FOLDER_CARD_OPEN_DELAY_MS = 300;
export const FOLDER_CARD_CLOSE_GRACE_MS = 150;
/** How far round the point where the pointer left one surface its way to the other one starts. */
const GRACE_PADDING_PX = 8;

type CardState = { open: boolean; pinned: boolean };
const CLOSED: CardState = { open: false, pinned: false };
const HOVER_OPEN: CardState = { open: true, pinned: false };
const PINNED: CardState = { open: true, pinned: true };

export type Point = { x: number; y: number };
type Box = Pick<DOMRect, 'top' | 'right' | 'bottom' | 'left'>;

/** What Radix hands the content's focus and outside-interaction callbacks. */
type PreventableEvent = { preventDefault(): void; target: EventTarget | null };

/** What Radix positions the card against (`PopoverAnchor`'s `virtualRef`). */
export type FolderCardAnchor = {
  getBoundingClientRect(): DOMRect;
  /** The element whose scrolling, resizing and moving re-place the card. */
  readonly contextElement?: Element;
};

/**
 * The box the card hangs from: the chip, stretched up to the top of the composer card it sits
 * under. With `side="top"` that puts the card above the composer, so a card a hover opened never
 * covers the draft or its controls; the chip's own left edge and width stay, so `align="start"`
 * still lines the card up with the chip. Without a composer (or before it has a layout) it is the
 * chip itself.
 */
export function folderCardAnchorRect(chip: DOMRect, composer: DOMRect | null): DOMRect {
  if (!composer || (composer.width === 0 && composer.height === 0)) return chip;
  const top = Math.min(chip.top, composer.top);
  return new DOMRect(chip.left, top, chip.width, chip.bottom - top);
}

function corners(box: Box): Point[] {
  return [
    { x: box.left, y: box.top },
    { x: box.right, y: box.top },
    { x: box.right, y: box.bottom },
    { x: box.left, y: box.bottom },
  ];
}

const cross = (o: Point, a: Point, b: Point) =>
  (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);

/** The convex hull of the points (Andrew's monotone chain). */
function convexHull(points: Point[]): Point[] {
  const sorted = [...points].sort((a, b) => a.x - b.x || a.y - b.y);
  if (sorted.length < 3) return sorted;
  const lower: Point[] = [];
  for (const point of sorted) {
    while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], point) <= 0)
      lower.pop();
    lower.push(point);
  }
  const upper: Point[] = [];
  for (const point of [...sorted].reverse()) {
    while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], point) <= 0)
      upper.pop();
    upper.push(point);
  }
  lower.pop();
  upper.pop();
  return [...lower, ...upper];
}

/**
 * The pointer's way from where it left one surface to the other: the hull of a small square round
 * the exit point and the whole of the other surface. A pointer moving inside it is on its way; one
 * that strays out of it is not.
 */
export function pointerGraceArea(exit: Point, target: Box, padding = GRACE_PADDING_PX): Point[] {
  return convexHull([
    ...corners({
      top: exit.y - padding,
      right: exit.x + padding,
      bottom: exit.y + padding,
      left: exit.x - padding,
    }),
    ...corners(target),
  ]);
}

/** Ray casting: whether the point lies inside the polygon. */
export function isPointInPolygon(point: Point, polygon: Point[]): boolean {
  let inside = false;
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const a = polygon[i];
    const b = polygon[j];
    if (
      a.y > point.y !== b.y > point.y &&
      point.x < ((b.x - a.x) * (point.y - a.y)) / (b.y - a.y) + a.x
    )
      inside = !inside;
  }
  return inside;
}

export interface FolderCard {
  /** Whether the card shows (never while suppressed). */
  open: boolean;
  /** Whether a press inside the card or the locked chip's click holds it open. */
  pinned: boolean;
  /** The chip: hover and focus open from here, and focus returns here. */
  triggerRef: React.RefCallback<HTMLElement>;
  /** The card's surface. */
  contentRef: React.RefObject<HTMLDivElement | null>;
  /** What the card is placed against: see `folderCardAnchorRect`. */
  anchor: React.RefObject<FolderCardAnchor>;
  triggerHandlers: {
    onPointerEnter: (event: React.PointerEvent) => void;
    onPointerLeave: (event: React.PointerEvent) => void;
    onFocus: () => void;
    onBlur: (event: React.FocusEvent) => void;
  };
  contentHandlers: {
    onPointerEnter: (event: React.PointerEvent) => void;
    onPointerLeave: (event: React.PointerEvent) => void;
    onPointerDown: () => void;
    onKeyDown: (event: React.KeyboardEvent) => void;
    onOpenAutoFocus: (event: Event) => void;
    onCloseAutoFocus: (event: Event) => void;
    onInteractOutside: (event: PreventableEvent) => void;
  };
  /** The Popover root's `onOpenChange`: Radix only ever asks to close (Escape, outside). */
  onOpenChange: (open: boolean) => void;
  /** The locked chip's click: pin it open, or close a pinned card. */
  togglePin: (options: { moveFocus: boolean }) => void;
  close: () => void;
}

export function useFolderCard({
  suppressed,
  clearOf,
}: {
  suppressed: boolean;
  /**
   * The composer card the chip sits under. The card hangs above it rather than over it; without
   * one, the card hangs from the chip.
   */
  clearOf?: React.RefObject<HTMLElement | null>;
}): FolderCard {
  const trigger = React.useRef<HTMLElement | null>(null);
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const [state, setState] = React.useState<CardState>(CLOSED);
  const openTimer = React.useRef<number | null>(null);
  const closeTimer = React.useRef<number | null>(null);
  /** A keyboard pin asked for focus before the card had mounted: its open auto-focus does it. */
  const focusOnOpen = React.useRef(false);
  /** Between a press inside the card and its release: the chip's blur then is that press's. */
  const pressInCard = React.useRef(false);
  /** While `close` hands focus back to the chip: that focus is a program's, and opens nothing. */
  const restoringFocus = React.useRef(false);
  /** An outside press or focus is dismissing the card: focus stays where it put it. */
  const dismissedFromOutside = React.useRef(false);
  /** The pointer's way across, while it is between the chip and the card. */
  const transit = React.useRef<{ onMove: (event: PointerEvent) => void } | null>(null);
  const suppressedNow = React.useRef(suppressed);
  const clearOfNow = React.useRef(clearOf);

  React.useEffect(() => {
    clearOfNow.current = clearOf;
  }, [clearOf]);

  const anchor = React.useRef<FolderCardAnchor>({
    getBoundingClientRect: () =>
      trigger.current
        ? folderCardAnchorRect(
            trigger.current.getBoundingClientRect(),
            clearOfNow.current?.current?.getBoundingClientRect() ?? null
          )
        : new DOMRect(),
    // The composer when there is one, so a composer that grows as the draft does moves the card
    // up with it.
    get contextElement() {
      return clearOfNow.current?.current ?? trigger.current ?? undefined;
    },
  });

  const triggerRef = React.useCallback((node: HTMLElement | null) => {
    trigger.current = node;
  }, []);

  const clearTimers = React.useCallback(() => {
    if (openTimer.current !== null) window.clearTimeout(openTimer.current);
    if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    openTimer.current = null;
    closeTimer.current = null;
  }, []);

  const endTransit = React.useCallback(() => {
    const current = transit.current;
    if (!current) return;
    transit.current = null;
    document.removeEventListener('pointermove', current.onMove, true);
  }, []);

  React.useEffect(
    () => () => {
      clearTimers();
      endTransit();
    },
    [clearTimers, endTransit]
  );

  const close = React.useCallback(() => {
    clearTimers();
    endTransit();
    focusOnOpen.current = false;
    const fromOutside = dismissedFromOutside.current;
    dismissedFromOutside.current = false;
    // Focus goes back to the chip only when it was inside the card and the close was the
    // person's own (Escape, Shift+Tab, the chip): an outside press is still on its way to where
    // it lands, and taking focus to the chip first would fire focus handlers it never meant to.
    const content = contentRef.current;
    const focusWasInside = !!content && content.contains(document.activeElement);
    setState(CLOSED);
    if (!focusWasInside || fromOutside || !trigger.current) return;
    restoringFocus.current = true;
    trigger.current.focus();
    queueMicrotask(() => {
      restoringFocus.current = false;
    });
  }, [clearTimers, endTransit]);

  const cancelClose = React.useCallback(() => {
    if (closeTimer.current !== null) {
      window.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
  }, []);

  const scheduleClose = React.useCallback(() => {
    if (openTimer.current !== null) {
      window.clearTimeout(openTimer.current);
      openTimer.current = null;
    }
    if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    closeTimer.current = window.setTimeout(() => {
      closeTimer.current = null;
      endTransit();
      setState((current) => (current.pinned ? current : CLOSED));
    }, FOLDER_CARD_CLOSE_GRACE_MS);
  }, [endTransit]);

  /** The pointer left one surface for the other: keep the grace alive while it is on its way. */
  const startTransit = (exit: Point, toward: Element | null) => {
    endTransit();
    if (!toward) return;
    const area = pointerGraceArea(exit, toward.getBoundingClientRect());
    const onMove = (event: PointerEvent) => {
      if (event.pointerType === 'touch') return;
      if (isPointInPolygon({ x: event.clientX, y: event.clientY }, area)) scheduleClose();
      else endTransit();
    };
    transit.current = { onMove };
    document.addEventListener('pointermove', onMove, true);
  };

  // The native chooser is up: the card is shut, and nothing armed before or during it opens the
  // card after it closes (#50). `open` below already hides it for this render.
  React.useEffect(() => {
    suppressedNow.current = suppressed;
    if (!suppressed) return;
    clearTimers();
    endTransit();
    focusOnOpen.current = false;
    setState(CLOSED);
  }, [suppressed, clearTimers, endTransit]);

  // A scroll that moves the chip closes an unpinned card (Radix Tooltip's rule); a scroll inside
  // the card, or in a scroller beside the chip, leaves it alone.
  React.useEffect(() => {
    if (!state.open || state.pinned) return;
    const onScroll = (event: Event) => {
      const target = event.target;
      const chip = trigger.current;
      if (chip && target instanceof Node && !target.contains(chip)) return;
      close();
    };
    window.addEventListener('scroll', onScroll, { capture: true, passive: true });
    return () => window.removeEventListener('scroll', onScroll, { capture: true });
  }, [state.open, state.pinned, close]);

  const isInCard = (node: EventTarget | null) =>
    node instanceof Node && !!contentRef.current?.contains(node);

  const triggerHandlers: FolderCard['triggerHandlers'] = {
    onPointerEnter: (event) => {
      if (event.pointerType === 'touch' || suppressed) return;
      cancelClose();
      endTransit();
      if (state.open || openTimer.current !== null) return;
      openTimer.current = window.setTimeout(() => {
        openTimer.current = null;
        if (suppressedNow.current) return;
        setState((current) => (current.open ? current : HOVER_OPEN));
      }, FOLDER_CARD_OPEN_DELAY_MS);
    },
    onPointerLeave: (event) => {
      if (event.pointerType === 'touch') return;
      scheduleClose();
      if (state.open && !state.pinned) {
        startTransit({ x: event.clientX, y: event.clientY }, contentRef.current);
      }
    },
    onFocus: () => {
      if (suppressed || restoringFocus.current || !isFocusFromTabKey()) return;
      clearTimers();
      setState((current) => (current.open ? current : HOVER_OPEN));
    },
    onBlur: (event) => {
      if (isInCard(event.relatedTarget) || pressInCard.current) return;
      close();
    },
  };

  const contentHandlers: FolderCard['contentHandlers'] = {
    onPointerEnter: (event) => {
      if (event.pointerType === 'touch') return;
      cancelClose();
      endTransit();
    },
    onPointerLeave: (event) => {
      if (event.pointerType === 'touch') return;
      scheduleClose();
      if (!state.pinned) startTransit({ x: event.clientX, y: event.clientY }, trigger.current);
    },
    onPointerDown: () => {
      pressInCard.current = true;
      const release = () => {
        pressInCard.current = false;
        document.removeEventListener('pointerup', release, true);
        document.removeEventListener('pointercancel', release, true);
      };
      document.addEventListener('pointerup', release, true);
      document.addEventListener('pointercancel', release, true);
      cancelClose();
      endTransit();
      setState((current) => (current.open ? PINNED : current));
    },
    onKeyDown: (event) => {
      // Shift+Tab off the card's own surface (where a keyboard pin, or a press on its background,
      // puts focus) goes back to the chip. Radix's focus loop wraps only from the first or last
      // control, so from the surface the browser would carry focus past the card to the end of
      // the page, which dismisses the card and strands the person there.
      if (event.key !== 'Tab' || !event.shiftKey || event.altKey || event.ctrlKey || event.metaKey)
        return;
      if (event.target !== contentRef.current) return;
      event.preventDefault();
      close();
    },
    onOpenAutoFocus: (event) => {
      // Hover and Tab opens leave focus on the chip. A keyboard pin moves it to the card itself,
      // the dialog, whose name is read out; the next Tab reaches its first control.
      event.preventDefault();
      if (!focusOnOpen.current) return;
      focusOnOpen.current = false;
      contentRef.current?.focus();
    },
    // `close` decides where focus goes; Radix would otherwise send it to a trigger it never had.
    onCloseAutoFocus: (event) => event.preventDefault(),
    onInteractOutside: (event) => {
      // A press on the chip, or focus returning to it, belongs to the chip's own handlers.
      const target = event.target;
      if (target instanceof Node && trigger.current?.contains(target)) {
        event.preventDefault();
        return;
      }
      // Radix dismisses next, in this same call: `close` then leaves focus where this press or
      // focus puts it. Cleared after this task, in case Radix declines to dismiss after all.
      dismissedFromOutside.current = true;
      queueMicrotask(() => {
        dismissedFromOutside.current = false;
      });
    },
  };

  const togglePin: FolderCard['togglePin'] = ({ moveFocus }) => {
    clearTimers();
    endTransit();
    if (state.open && state.pinned) {
      close();
      return;
    }
    setState(PINNED);
    if (!moveFocus) return;
    const content = contentRef.current;
    // Already on screen (a hover or a Tab opened it): straight in. Otherwise the card's open
    // auto-focus does it once the surface exists.
    if (content) content.focus();
    else focusOnOpen.current = true;
  };

  const onOpenChange = (next: boolean) => {
    if (!next) close();
  };

  return {
    open: state.open && !suppressed,
    pinned: state.pinned,
    triggerRef,
    contentRef,
    anchor,
    triggerHandlers,
    contentHandlers,
    onOpenChange,
    togglePin,
    close,
  };
}
