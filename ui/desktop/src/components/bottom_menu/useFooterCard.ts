import * as React from 'react';
import { isFocusFromTabKey } from '../ui/Tooltip';

/**
 * When a footer card opens and closes, and where it hangs: ONE behaviour for every item on the
 * composer's footer line (the working folder, the context ring, the cost), so the three answer a
 * hover, a click, a key and a scroll the same way. The surface itself is `FooterCard.tsx`.
 *
 * The rules are InfoTip's (`ui/info-tip.tsx`), at a hover card's pace, with the differences that
 * come from what a card holds (buttons, a slider, in a browser a key field) and from where it
 * hangs: ABOVE the whole composer card, not on its trigger, so a card a hover opened never covers
 * the draft, `+` or Tools (`footerCardAnchorRect`). So
 *   - mouse or pen hover opens it after {@link FOOTER_CARD_OPEN_DELAY_MS}; touch never does.
 *     Leaving the trigger or the card closes it after {@link FOOTER_CARD_CLOSE_GRACE_MS}; a
 *     pointer still MOVING along its way to the other one (`pointerGraceArea`) re-arms that
 *     grace, because the way up from the footer crosses the composer. Entering either one cancels
 *     it, and a pointer that stops on the way, or strays off it, lets it run out;
 *   - a Tab onto the trigger opens it at once (Tooltip's gate, so a focus a program restores never
 *     does, including the one `close` makes). The trigger's blur closes an UNPINNED card unless
 *     focus went into the card. A pinned card outlives that blur: switching windows blurs the
 *     trigger with nowhere for focus to go, and closed a card the person had pinned. Focus that
 *     really moves elsewhere in the page still closes it, through Radix's own focus-outside
 *     dismissal;
 *   - Escape and a press outside close it (a press on the trigger is not outside). Focus goes back
 *     to the trigger only for a close the person made from the keyboard or the trigger: Escape,
 *     Shift+Tab off the card's own surface, the trigger's click. An outside press or focus leaves
 *     focus where that interaction puts it, as Radix's own non-modal popover does, so the press
 *     that closed the card never hands focus to the trigger mid-click;
 *   - a PRESS inside the card pins it, so clicking a button in it or typing in a field never loses
 *     the card to a pointer that wandered off; a pinned card ignores pointer leave and scroll. So
 *     does FOCUS that enters it: the cards are portalled to the end of the page, so a Tab past the
 *     last item on the line (the folder chip on Home) lands in its card, and a card that holds
 *     focus must not be closed by a pointer's grace running out, which would drop focus on the
 *     page;
 *   - `togglePin` is every trigger's click (and a tap, since touch never hovers): it pins the card,
 *     and a second click closes it. A keyboard activation also moves focus into the card. A Tab
 *     from the card's own surface reaches its first control; in a card that holds none (the cost
 *     card) it closes the card and carries on from the trigger, rather than doing nothing;
 *   - ONE CARD AT A TIME: inside a `FooterCardGroup` (the footer line), a card that opens, by
 *     hover, focus or pin, closes every other card in the group at once. Focus is never moved to
 *     the card that opened; a card that closes while it holds focus hands it back to its own
 *     trigger, as any close of it would, rather than dropping it on the page;
 *   - while `suppressed` (the folder chip's native chooser is open) it is shut, and nothing a
 *     hover, a focus or a click armed during that time opens it once the chooser closes (#50).
 *
 * InfoTip itself is untouched: it shares the approach, not the code, so its behaviour cannot move.
 */

export const FOOTER_CARD_OPEN_DELAY_MS = 300;
export const FOOTER_CARD_CLOSE_GRACE_MS = 150;
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
export type FooterCardAnchor = {
  getBoundingClientRect(): DOMRect;
  /** The element whose scrolling, resizing and moving re-place the card. */
  readonly contextElement?: Element;
};

/**
 * The box a card hangs from: its trigger, stretched up to the top of the composer card the footer
 * sits under. With `side="top"` that puts the card above the composer, so a card a hover opened
 * never covers the draft or its controls; the trigger's own left edge and width stay, so
 * `align="start"` lines the card up with the trigger's left and `align="end"` with its right.
 * Without a composer (or before it has a layout) it is the trigger itself.
 */
export function footerCardAnchorRect(trigger: DOMRect, composer: DOMRect | null): DOMRect {
  if (!composer || (composer.width === 0 && composer.height === 0)) return trigger;
  const top = Math.min(trigger.top, composer.top);
  return new DOMRect(trigger.left, top, trigger.width, trigger.bottom - top);
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

/**
 * Whether anything in the card can take a Tab: Radix's own test (`FocusScope`'s tabbable
 * candidates and its `isHidden`), so this agrees with the focus loop that wraps Tab inside it.
 */
export function holdsTabbable(container: HTMLElement): boolean {
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_ELEMENT, {
    acceptNode: (node) => {
      const element = node as HTMLElement & { disabled?: boolean; type?: string };
      const isHiddenInput = element.tagName === 'INPUT' && element.type === 'hidden';
      if (element.disabled || element.hidden || isHiddenInput) return NodeFilter.FILTER_SKIP;
      return element.tabIndex >= 0 ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
    },
  });
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const element = node as HTMLElement;
    if (getComputedStyle(element).visibility === 'hidden') continue;
    let shown = true;
    for (let up: HTMLElement | null = element; up && up !== container; up = up.parentElement) {
      if (getComputedStyle(up).display === 'none') {
        shown = false;
        break;
      }
    }
    if (shown) return true;
  }
  return false;
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

/**
 * What the footer line shares with each of its cards (`FooterCardGroup`, in `FooterCard.tsx`): the
 * composer card they all hang above, and the rule that only one of them is open at a time.
 */
export interface FooterCardGroupValue {
  /** The composer card above the line. Every card hangs above it; without one, from its trigger. */
  clearOf?: React.RefObject<HTMLElement | null>;
  /** A card in the group is opening: every other one closes. */
  opened(id: string): void;
  /** Adds a card's close to the group, until the returned function removes it. */
  register(id: string, close: () => void): () => void;
}

export const FooterCardGroupContext = React.createContext<FooterCardGroupValue | null>(null);

/** The group's value, kept stable for the life of the line: see `FooterCardGroup`. */
export function useFooterCardGroupValue(
  clearOf?: React.RefObject<HTMLElement | null>
): FooterCardGroupValue {
  const cards = React.useRef(new Map<string, () => void>());
  return React.useMemo(
    () => ({
      clearOf,
      opened: (id: string) => {
        for (const [other, close] of [...cards.current]) if (other !== id) close();
      },
      register: (id: string, close: () => void) => {
        cards.current.set(id, close);
        return () => {
          if (cards.current.get(id) === close) cards.current.delete(id);
        };
      },
    }),
    [clearOf]
  );
}

export interface FooterCard {
  /** Whether the card shows (never while suppressed). */
  open: boolean;
  /** Whether a press inside the card or the trigger's click holds it open. */
  pinned: boolean;
  /** The trigger: hover and focus open from here, and focus returns here. */
  triggerRef: React.RefCallback<HTMLElement>;
  /** The card's surface. */
  contentRef: React.RefObject<HTMLDivElement | null>;
  /** What the card is placed against: see `footerCardAnchorRect`. */
  anchor: React.RefObject<FooterCardAnchor>;
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
    onFocus: () => void;
    onKeyDown: (event: React.KeyboardEvent) => void;
    onOpenAutoFocus: (event: Event) => void;
    onCloseAutoFocus: (event: Event) => void;
    onInteractOutside: (event: PreventableEvent) => void;
  };
  /** The Popover root's `onOpenChange`: Radix only ever asks to close (Escape, outside). */
  onOpenChange: (open: boolean) => void;
  /** The trigger's click: pin the card open, or close a pinned card. */
  togglePin: (options: { moveFocus: boolean }) => void;
  close: () => void;
}

export function useFooterCard({ suppressed = false }: { suppressed?: boolean } = {}): FooterCard {
  const group = React.useContext(FooterCardGroupContext);
  const id = React.useId();
  const trigger = React.useRef<HTMLElement | null>(null);
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const [state, setState] = React.useState<CardState>(CLOSED);
  const openTimer = React.useRef<number | null>(null);
  const closeTimer = React.useRef<number | null>(null);
  /** A keyboard pin asked for focus before the card had mounted: its open auto-focus does it. */
  const focusOnOpen = React.useRef(false);
  /** Between a press inside the card and its release: the trigger's blur then is that press's. */
  const pressInCard = React.useRef(false);
  /** While `close` hands focus back to the trigger: that focus is a program's, and opens nothing. */
  const restoringFocus = React.useRef(false);
  /** An outside press or focus is dismissing the card: focus stays where it put it. */
  const dismissedFromOutside = React.useRef(false);
  /** The pointer's way across, while it is between the trigger and the card. */
  const transit = React.useRef<{ onMove: (event: PointerEvent) => void } | null>(null);
  const suppressedNow = React.useRef(suppressed);
  const clearOf = group?.clearOf;
  const clearOfNow = React.useRef(clearOf);

  React.useEffect(() => {
    clearOfNow.current = clearOf;
  }, [clearOf]);

  const anchor = React.useRef<FooterCardAnchor>({
    getBoundingClientRect: () =>
      trigger.current
        ? footerCardAnchorRect(
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
    // Focus goes back to the trigger only when it was inside the card and the close was the
    // person's own (Escape, Shift+Tab, the trigger): an outside press is still on its way to where
    // it lands, and taking focus to the trigger first would fire focus handlers it never meant to.
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

  // One card at a time: the group closes this one when another opens.
  React.useEffect(() => group?.register(id, close), [group, id, close]);

  /** This card is opening: the group's others close first, so two are never on screen. */
  const claim = () => group?.opened(id);

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
    }, FOOTER_CARD_CLOSE_GRACE_MS);
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

  // A scroll that moves the trigger closes an unpinned card (Radix Tooltip's rule); a scroll
  // inside the card, or in a scroller beside the trigger, leaves it alone.
  React.useEffect(() => {
    if (!state.open || state.pinned) return;
    const onScroll = (event: Event) => {
      const target = event.target;
      const node = trigger.current;
      if (node && target instanceof Node && !target.contains(node)) return;
      close();
    };
    window.addEventListener('scroll', onScroll, { capture: true, passive: true });
    return () => window.removeEventListener('scroll', onScroll, { capture: true });
  }, [state.open, state.pinned, close]);

  const isInCard = (node: EventTarget | null) =>
    node instanceof Node && !!contentRef.current?.contains(node);

  const triggerHandlers: FooterCard['triggerHandlers'] = {
    onPointerEnter: (event) => {
      if (event.pointerType === 'touch' || suppressed) return;
      cancelClose();
      endTransit();
      if (state.open || openTimer.current !== null) return;
      openTimer.current = window.setTimeout(() => {
        openTimer.current = null;
        if (suppressedNow.current) return;
        claim();
        setState((current) => (current.open ? current : HOVER_OPEN));
      }, FOOTER_CARD_OPEN_DELAY_MS);
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
      claim();
      setState((current) => (current.open ? current : HOVER_OPEN));
    },
    onBlur: (event) => {
      if (isInCard(event.relatedTarget) || pressInCard.current) return;
      // A pinned card outlives its trigger's blur (a window switch blurs it with relatedTarget
      // null). Focus that really moved elsewhere in the page is Radix's to dismiss.
      if (state.pinned) return;
      close();
    },
  };

  const contentHandlers: FooterCard['contentHandlers'] = {
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
    onFocus: () => {
      // Focus inside the card pins it, however it got there. A press already has (above); this is
      // for the keyboard: the card is portalled to the end of the page, so a Tab past the last
      // item on the line (the folder chip on Home, the readout last on a chat's line) moves into
      // its card. Unpinned, that card would still close on the pointer's grace and drop focus on
      // the page, with the person's place in it gone.
      cancelClose();
      endTransit();
      setState((current) => (current.open && !current.pinned ? PINNED : current));
    },
    onKeyDown: (event) => {
      if (event.key !== 'Tab' || event.altKey || event.ctrlKey || event.metaKey) return;
      const content = contentRef.current;
      // Only a Tab from the card's own surface (where a keyboard pin, or a press on its
      // background, puts focus). From a control, Radix's focus loop and the browser decide.
      if (!content || event.target !== content) return;
      if (event.shiftKey) {
        // Shift+Tab goes back to the trigger. Radix's focus loop wraps only from the first or
        // last control, so from the surface the browser would carry focus past the card to the end
        // of the page, which dismisses the card and strands the person there.
        event.preventDefault();
        close();
        return;
      }
      // A forward Tab reaches the card's first control. A card that holds none (the cost card, a
      // locked chat's folder card with no grants) would swallow it: Radix's loop cancels a Tab on
      // a surface with nothing to wrap to, so nothing moves and nothing is said. Instead the card
      // closes, focus goes back to the trigger, and the Tab is left to the browser, which carries
      // on from where focus now is, the trigger, to whatever follows it. This runs before Radix's
      // own handler, which then finds focus off the surface and leaves the Tab alone.
      if (holdsTabbable(content)) return;
      close();
    },
    onOpenAutoFocus: (event) => {
      // Hover and Tab opens leave focus on the trigger. A keyboard pin moves it to the card
      // itself, the dialog, whose name is read out; the next Tab reaches its first control.
      event.preventDefault();
      if (!focusOnOpen.current) return;
      focusOnOpen.current = false;
      contentRef.current?.focus();
    },
    // `close` decides where focus goes; Radix would otherwise send it to a trigger it never had.
    onCloseAutoFocus: (event) => event.preventDefault(),
    onInteractOutside: (event) => {
      // A press on the trigger, or focus returning to it, belongs to the trigger's own handlers.
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

  const togglePin: FooterCard['togglePin'] = ({ moveFocus }) => {
    // #50: a click while the chooser is up arms nothing for after it.
    if (suppressed) return;
    clearTimers();
    endTransit();
    if (state.open && state.pinned) {
      close();
      return;
    }
    claim();
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

/**
 * Everything a footer item's trigger carries, spelled once for all three (the folder chip, the
 * context ring, the cost): the card's open state for assistive technology, the hook's hover and
 * focus handlers, and the click that pins. `detail` is 0 for Enter, Space and an assistive
 * technology's click: those also move focus into the card, so its controls can be reached from
 * the keyboard. `FooterCardTrigger` (FooterCard.tsx) renders the button with these.
 */
export function footerCardTriggerProps(card: FooterCard, cardId: string) {
  return {
    ref: card.triggerRef,
    'aria-haspopup': 'dialog' as const,
    'aria-expanded': card.open,
    'aria-controls': card.open ? cardId : undefined,
    ...card.triggerHandlers,
    onClick: (event: React.MouseEvent) => card.togglePin({ moveFocus: event.detail === 0 }),
  };
}
