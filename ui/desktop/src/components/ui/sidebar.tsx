'use client';

import * as React from 'react';
import { Slot } from '@radix-ui/react-slot';
import { VariantProps, cva } from 'class-variance-authority';
import { PanelLeftIcon } from '../icons/app-icons';

import { cn } from '../../utils';
import { Button } from './button';
import { Input } from './input';
import { Separator } from './separator';
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from './sheet';
import { Skeleton } from './skeleton';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './Tooltip';
import { useIsMobile } from '../../hooks/use-mobile';
import {
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
  SIDEBAR_WIDTH_KEYBOARD_STEP,
  clampSidebarWidth,
  readStoredSidebarWidth,
  writeStoredSidebarWidth,
} from './sidebarWidth';

const SIDEBAR_COOKIE_NAME = 'sidebar_state';
const SIDEBAR_COOKIE_MAX_AGE = 60 * 60 * 24 * 7;
/**
 * The mobile drawer stays at the canonical default and is NOT resizable.
 *
 * It is a sheet over a narrow window, not a column beside the content: there is
 * no edge to drag (the drag handle is desktop-only), and following a width the
 * user chose on a wide window would size an overlay by a decision made about a
 * layout that is not on screen. `sidebar.test.tsx` pins this deliberately.
 */
const SIDEBAR_WIDTH_MOBILE = `${SIDEBAR_DEFAULT_WIDTH}px`;
const SIDEBAR_WIDTH_ICON = '38px';
const SIDEBAR_KEYBOARD_SHORTCUT = 'b';
/** Marks the drag on `<body>` so the width transitions stop lagging the pointer. */
const SIDEBAR_RESIZING_CLASS = 'biorouter-sidebar-resizing';
/**
 * The class that makes an open sidebar an OVERLAY rather than a column.
 *
 * `AppLayout` puts it on `<body>` below rung 1 of the yield ladder
 * (`SIDEBAR_COMPACT_WIDTH`), and `main.css` keys the overlay on it: the gap
 * collapses, the inset stops making room and the panel floats over the page.
 * The class IS the presentation, so it is also the test for "the user is
 * looking at an overlay" — a second width rule here could only drift from the
 * one that actually draws it. `sidebar.test.tsx` pins all three places to it.
 */
export const SIDEBAR_OVERLAY_BODY_CLASS = 'biorouter-sidebar-compact';
/**
 * What a person activates to CHOOSE something in the sidebar: a destination, a
 * chat, an action. A disclosure or a menu opener is not a choice (it changes what
 * the sidebar shows, and the user is still choosing), so it is excluded below by
 * its ARIA state rather than listed here.
 */
const SIDEBAR_CHOICE_SELECTOR =
  'a[href], button, [role="button"], [role="link"], [role="menuitem"], [role="option"]';

function sidebarIsOverlay(): boolean {
  return document.body.classList.contains(SIDEBAR_OVERLAY_BODY_CLASS);
}

/**
 * Whether a click inside the panel chose something, so an overlay should get
 * out of the way of what was chosen (triage T-66).
 *
 * The DOM containment check is load-bearing: a context menu or a confirmation
 * opened from a row is portalled to `<body>`, and React still bubbles its clicks
 * through the row — but acting in that menu is not picking a destination here.
 */
function isSidebarChoice(target: EventTarget | null, panel: HTMLElement): boolean {
  if (!(target instanceof Element)) return false;
  const control = target.closest(SIDEBAR_CHOICE_SELECTOR);
  if (!control || !panel.contains(control)) return false;
  if (control.hasAttribute('aria-expanded') || control.hasAttribute('aria-haspopup')) return false;
  return !control.matches(':disabled, [aria-disabled="true"]');
}

/** The panel's first keyboard stop, skipping the resize edge, which sits last anyway. */
const SIDEBAR_FOCUSABLE =
  'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]';

function firstFocusableIn(panel: HTMLElement): HTMLElement | null {
  for (const element of panel.querySelectorAll<HTMLElement>(SIDEBAR_FOCUSABLE)) {
    if (element.tabIndex < 0) continue;
    if (element.closest('[inert], [hidden], [aria-hidden="true"]')) continue;
    if (element.matches('[data-slot="sidebar-resize-handle"]')) continue;
    return element;
  }
  return null;
}

/**
 * Put focus on the page's content region: the route's `<main>` inside the inset, or the inset
 * itself when a layout supplies no landmark. For a choice made in the overlay (live QA round 4,
 * Q4-54): the panel is going inert with focus on the row that was chosen, and the toggle is the
 * wrong place to hand it — the person asked for the page. The destination's own focus (a
 * composer, a field) lands in a later effect and wins; a destination that focuses nothing
 * leaves focus here, so the next Tab walks into the page rather than back along the titlebar.
 *
 * `<main>` is not focusable by itself, so it gets `tabindex="-1"` for exactly as long as it
 * holds this focus, and loses it when focus moves on — a lasting `tabindex` would make every
 * click on the page's blank space focus the whole region. Its outline is held back meanwhile:
 * like a tab panel (D-15 in `main.css`), a region the next Tab leaves for a control inside
 * gets no ring of its own, and the UA's `outline: auto` would draw one around the whole page.
 * The `prefers-contrast` / `forced-colors` ring is `!important` and still draws, as it does on
 * every `[tabindex]`.
 *
 * Returns false when there is no region, or it would not take focus.
 */
function focusContentRegion(): boolean {
  const inset = document.querySelector<HTMLElement>('[data-slot="sidebar-inset"]');
  const region = inset?.querySelector<HTMLElement>('main') ?? inset;
  if (!region || region.closest('[inert]')) return false;

  const lent = !region.hasAttribute('tabindex');
  const outlineBefore = region.style.getPropertyValue('outline');
  const release = () => {
    // The window losing focus blurs the region too, but focus comes back to it.
    if (document.activeElement === region) return;
    region.removeEventListener('blur', release);
    region.removeAttribute('tabindex');
    if (outlineBefore) region.style.setProperty('outline', outlineBefore);
    else region.style.removeProperty('outline');
  };
  if (lent) {
    region.setAttribute('tabindex', '-1');
    region.style.setProperty('outline', 'none');
  }
  region.focus({ preventScroll: true });
  if (document.activeElement === region) {
    if (lent) region.addEventListener('blur', release);
    return true;
  }
  if (lent) release();
  return false;
}

type SidebarContextProps = {
  state: 'expanded' | 'collapsed';
  open: boolean;
  setOpen: (open: boolean) => void;
  openMobile: boolean;
  setOpenMobile: (open: boolean) => void;
  isMobile: boolean;
  toggleSidebar: () => void;
  /** The current sidebar width in px, already clamped to the module's bounds. */
  width: number;
  /** True for the duration of a pointer drag on the resize handle. */
  isResizing: boolean;
  /** Begin a pointer drag. Wired to the handle's `onPointerDown`. */
  startResize: (event: React.PointerEvent<HTMLElement>) => void;
  /** Move the edge by a signed number of px, clamped and persisted. */
  nudgeWidth: (delta: number) => void;
  /** Restore the default width (the handle's double-click). */
  resetWidth: () => void;
  /**
   * Set by the ⌘B / Ctrl+B shortcut, for the one render its toggle causes: what had focus when
   * it was pressed. The panel reads it to move focus into an overlay the shortcut opened, and to
   * hand focus back when the shortcut closes it (Q2-51). `null` means no shortcut is pending.
   */
  shortcutToggleRef: React.RefObject<SidebarShortcutToggle | null>;
};

/** A ⌘B press, remembered long enough for the panel to act on the toggle it caused. */
type SidebarShortcutToggle = { focusedBefore: Element | null };

const SidebarContext = React.createContext<SidebarContextProps | null>(null);

function useSidebar() {
  const context = React.useContext(SidebarContext);
  if (!context) {
    throw new Error('useSidebar must be used within a SidebarProvider.');
  }

  return context;
}

function SidebarProvider({
  defaultOpen = true,
  open: openProp,
  onOpenChange: setOpenProp,
  className,
  style,
  children,
  ...props
}: React.ComponentProps<'div'> & {
  defaultOpen?: boolean;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  const isMobile = useIsMobile();
  const [openMobile, setOpenMobile] = React.useState(false);

  // This is the internal state of the sidebar.
  // We use openProp and setOpenProp for control from outside the component.
  const [_open, _setOpen] = React.useState(defaultOpen);
  const open = openProp ?? _open;
  const setOpen = React.useCallback(
    (value: boolean | ((value: boolean) => boolean)) => {
      const openState = typeof value === 'function' ? value(open) : value;
      if (setOpenProp) {
        setOpenProp(openState);
      } else {
        _setOpen(openState);
      }

      // This sets the cookie to keep the sidebar state.
      document.cookie = `${SIDEBAR_COOKIE_NAME}=${openState}; path=/; max-age=${SIDEBAR_COOKIE_MAX_AGE}`;
    },
    [setOpenProp, open]
  );

  // Helper to toggle the sidebar.
  const toggleSidebar = React.useCallback(() => {
    return isMobile ? setOpenMobile((open) => !open) : setOpen((open) => !open);
  }, [isMobile, setOpen, setOpenMobile]);

  const shortcutToggleRef = React.useRef<SidebarShortcutToggle | null>(null);

  // Adds a keyboard shortcut to toggle the sidebar.
  React.useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === SIDEBAR_KEYBOARD_SHORTCUT && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        // The panel acts on this in the layout effect of the render the toggle causes, which
        // React flushes before any timer; a toggle that changed nothing (the mobile sheet, a
        // controlled parent that refused) must not leave it for a later, unrelated one.
        const toggle: SidebarShortcutToggle = { focusedBefore: document.activeElement };
        shortcutToggleRef.current = toggle;
        window.setTimeout(() => {
          if (shortcutToggleRef.current === toggle) shortcutToggleRef.current = null;
        }, 0);
        toggleSidebar();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [toggleSidebar]);

  // We add a state so that we can do data-state="expanded" or "collapsed".
  // This makes it easier to style the sidebar with Tailwind classes.
  const state = open ? 'expanded' : 'collapsed';

  // ---------------------------------------------------------------------------
  // Width
  //
  // Modelled on `useArtifactPanel`'s splitter rather than invented a second time:
  // pointer capture with global listeners as the fallback, one rAF-batched write
  // per frame, and a single `finishResize` that every exit path funnels through
  // (pointerup, pointercancel, lost capture, window blur, unmount). That last
  // part is the whole reason the panel's version is shaped this way — a drag
  // that ends off-window otherwise leaves `col-resize` painted on the body and
  // a live pointermove listener behind it.
  //
  // The arithmetic itself is NOT here. It lives in `./sidebarWidth`, pure, so
  // the bounds are provable without rendering — jsdom computes no layout, so a
  // test that mounts this component can never see how wide the sidebar actually
  // got.
  // ---------------------------------------------------------------------------
  const [width, setWidth] = React.useState<number>(() => readStoredSidebarWidth());
  const [isResizing, setIsResizing] = React.useState(false);
  const resizeFrameRef = React.useRef<number | null>(null);
  const pendingWidthRef = React.useRef<number | null>(null);
  const resizeCleanupRef = React.useRef<(() => void) | null>(null);

  React.useEffect(() => {
    return () => {
      if (resizeFrameRef.current !== null) window.cancelAnimationFrame(resizeFrameRef.current);
      resizeCleanupRef.current?.();
    };
  }, []);

  const nudgeWidth = React.useCallback((delta: number) => {
    setWidth((current) => {
      const next = clampSidebarWidth(current + delta);
      writeStoredSidebarWidth(next);
      return next;
    });
  }, []);

  const resetWidth = React.useCallback(() => {
    setWidth(SIDEBAR_DEFAULT_WIDTH);
    writeStoredSidebarWidth(SIDEBAR_DEFAULT_WIDTH);
  }, []);

  const startResize = React.useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      event.preventDefault();
      event.stopPropagation();

      // A second pointer landing mid-drag must not run two loops over one width.
      resizeCleanupRef.current?.();

      const startX = event.clientX;
      const startWidth = width;
      const previousCursor = document.body.style.cursor;
      const previousUserSelect = document.body.style.userSelect;
      const handle = event.currentTarget;
      const pointerId = event.pointerId;
      let finished = false;
      let latestWidth = startWidth;

      try {
        handle.setPointerCapture(pointerId);
      } catch {
        // Capture is unavailable (jsdom, some pen devices). The window-level
        // listeners below keep the drag working without it.
      }

      setIsResizing(true);
      pendingWidthRef.current = null;
      document.body.classList.add(SIDEBAR_RESIZING_CLASS);
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';

      const applyPendingWidth = () => {
        resizeFrameRef.current = null;
        const nextWidth = pendingWidthRef.current;
        pendingWidthRef.current = null;
        if (nextWidth !== null) setWidth(nextWidth);
      };

      const scheduleWidth = (nextWidth: number) => {
        latestWidth = nextWidth;
        pendingWidthRef.current = nextWidth;
        if (resizeFrameRef.current !== null) return;
        resizeFrameRef.current = window.requestAnimationFrame(applyPendingWidth);
      };

      const handleMove = (moveEvent: globalThis.PointerEvent) => {
        if (moveEvent.pointerId !== pointerId) return;
        // The sidebar is on the LEFT, so rightward pointer movement widens it —
        // the opposite sign from the artifact panel's right-hand splitter.
        scheduleWidth(clampSidebarWidth(startWidth + (moveEvent.clientX - startX)));
      };

      const finishResize = (commitPendingWidth: boolean) => {
        if (finished) return;
        finished = true;
        if (resizeFrameRef.current !== null) {
          window.cancelAnimationFrame(resizeFrameRef.current);
          resizeFrameRef.current = null;
        }
        if (commitPendingWidth) {
          if (pendingWidthRef.current !== null) setWidth(pendingWidthRef.current);
          // Persisted once, at the end. A write per frame would put a synchronous
          // localStorage round-trip inside the drag loop.
          writeStoredSidebarWidth(latestWidth);
        }
        pendingWidthRef.current = null;
        setIsResizing(false);
        document.body.classList.remove(SIDEBAR_RESIZING_CLASS);
        document.body.style.cursor = previousCursor;
        document.body.style.userSelect = previousUserSelect;
        window.removeEventListener('pointermove', handleMove);
        window.removeEventListener('pointerup', handleEnd);
        window.removeEventListener('pointercancel', handleEnd);
        window.removeEventListener('blur', handleWindowBlur);
        handle.removeEventListener('lostpointercapture', handleLostPointerCapture);
        try {
          if (handle.hasPointerCapture(pointerId)) handle.releasePointerCapture(pointerId);
        } catch {
          // The handle may have left the document while the pointer was outside.
        }
        resizeCleanupRef.current = null;
      };

      const handleEnd = (endEvent: globalThis.PointerEvent) => {
        if (endEvent.pointerId !== pointerId) return;
        finishResize(true);
      };
      const handleWindowBlur = () => finishResize(true);
      const handleLostPointerCapture = (lostEvent: globalThis.PointerEvent) => {
        if (lostEvent.pointerId === pointerId) finishResize(true);
      };

      // Unmount mid-drag: drop the listeners and the body styles, but do NOT
      // persist — the width the user was mid-way through choosing is not one.
      resizeCleanupRef.current = () => finishResize(false);

      window.addEventListener('pointermove', handleMove);
      window.addEventListener('pointerup', handleEnd);
      window.addEventListener('pointercancel', handleEnd);
      window.addEventListener('blur', handleWindowBlur);
      handle.addEventListener('lostpointercapture', handleLostPointerCapture);
    },
    [width]
  );

  const contextValue = React.useMemo<SidebarContextProps>(
    () => ({
      state,
      open,
      setOpen,
      isMobile,
      openMobile,
      setOpenMobile,
      toggleSidebar,
      width,
      isResizing,
      startResize,
      nudgeWidth,
      resetWidth,
      shortcutToggleRef,
    }),
    [
      state,
      open,
      setOpen,
      isMobile,
      openMobile,
      setOpenMobile,
      toggleSidebar,
      width,
      isResizing,
      startResize,
      nudgeWidth,
      resetWidth,
    ]
  );

  return (
    <SidebarContext.Provider value={contextValue}>
      <TooltipProvider delayDuration={0}>
        <div
          data-slot="sidebar-wrapper"
          style={
            {
              '--sidebar-width': `${width}px`,
              '--sidebar-width-icon': SIDEBAR_WIDTH_ICON,
              ...style,
            } as React.CSSProperties
          }
          className={cn(
            'group/sidebar-wrapper has-data-[variant=inset]:bg-sidebar flex min-h-svh w-full',
            className
          )}
          {...props}
        >
          {children}
        </div>
      </TooltipProvider>
    </SidebarContext.Provider>
  );
}

function Sidebar({
  side = 'left',
  variant = 'sidebar',
  collapsible = 'offcanvas',
  className,
  children,
  ...props
}: React.ComponentProps<'div'> & {
  side?: 'left' | 'right';
  variant?: 'sidebar' | 'floating' | 'inset';
  collapsible?: 'offcanvas' | 'icon' | 'none';
}) {
  const { isMobile, state, open, setOpen, openMobile, setOpenMobile, shortcutToggleRef } =
    useSidebar();
  const panelRef = React.useRef<HTMLDivElement>(null);
  /** What had focus before ⌘B opened the overlay, to hand focus back to when ⌘B closes it. */
  const overlayReturnRef = React.useRef<Element | null>(null);
  /**
   * Set when a choice made in the overlay closes it, for the one render that close causes, so
   * the layout effect below hands focus to the page rather than the toggle (Q4-54). A token, as
   * with ⌘B: a close a controlled parent refused must not steer a later, unrelated one.
   */
  const choiceCloseRef = React.useRef<object | null>(null);

  /*
   * OFF-CANVAS MEANS OUT OF THE TAB ORDER (triage T-20). The collapsed panel is
   * only translated off-screen, so its seven controls stayed Tab stops at
   * x = -272: focus vanished for seven presses, and Enter on an invisible row
   * opened Settings by accident. `inert` takes the whole panel out of focus,
   * pointer and the accessibility tree while it is away, and opening it (⌘B,
   * the titlebar toggle) gives it all back. An `icon` sidebar keeps its rail on
   * screen, so it is never inert; the mobile sheet is a separate branch.
   */
  const offCanvas = collapsible === 'offcanvas' && state === 'collapsed';

  // A panel that goes inert while it holds focus would drop that focus on
  // <body>, and the next Tab would restart from the top of the document. Hand
  // it to the toggle that brings the panel back — the control the user would
  // reach for next. A route that focuses something itself (the composer) does
  // so in a later effect, and wins.
  //
  // ⌘B AND THE OVERLAY (live QA round 2, Q2-51). Below rung 1 an open sidebar
  // floats OVER the page, and ⌘B used to open it with focus left underneath:
  // erin's focus stayed on Crew's "Add channel", now hidden behind the panel,
  // and her next Tabs walked controls she could not see. So when the SHORTCUT
  // opens the overlay, focus moves to the panel's first stop; when the shortcut
  // closes it again, focus goes back to where it was before the panel opened.
  // A docked column covers nothing, so it takes no focus; a pointer toggle
  // leaves focus on the toggle the person just used.
  //
  // A CHOICE MADE IN THE OVERLAY HANDS FOCUS TO THE PAGE (live QA round 4, Q4-54).
  // It used to go to the toggle, like any other close, and erin arrived in Crew
  // with focus back up in the titlebar: one extra Tab, and her place lost on
  // arrival. The person asked for the page, so the page's content region takes
  // focus (`focusContentRegion`), and a destination that focuses something
  // itself does so in a later effect and wins. That region is the layout's
  // `<main>`, which outlives the route inside it, so it is there whatever the
  // choice replaced. The toggle stays the fallback for a layout with no region.
  React.useLayoutEffect(() => {
    const panel = panelRef.current;
    const shortcut = shortcutToggleRef.current;
    shortcutToggleRef.current = null;
    const chosen = choiceCloseRef.current !== null;
    choiceCloseRef.current = null;
    if (!panel) return;

    if (!offCanvas) {
      if (!shortcut || !sidebarIsOverlay()) return;
      const first = firstFocusableIn(panel);
      if (!first) return;
      overlayReturnRef.current = shortcut.focusedBefore;
      first.focus();
      return;
    }

    const returnTo = shortcut ? overlayReturnRef.current : null;
    overlayReturnRef.current = null;
    const focused = document.activeElement;
    if (!(focused instanceof HTMLElement) || !panel.contains(focused)) return;
    if (
      returnTo instanceof HTMLElement &&
      returnTo !== document.body &&
      returnTo.isConnected &&
      !panel.contains(returnTo)
    ) {
      returnTo.focus();
      if (document.activeElement === returnTo) return;
    }
    if (chosen && focusContentRegion()) return;
    document.querySelector<HTMLElement>('[data-sidebar="trigger"]')?.focus();
  }, [offCanvas, shortcutToggleRef]);

  // ESCAPE CLOSES THE OVERLAY (live QA round 3, Q3-60). Below rung 1 an open
  // sidebar is a floating surface over the page, and every other floating
  // surface in the app steps aside on Escape. This one did not: erin opened it
  // with the titlebar toggle (focus stays on the toggle, which is right), pressed
  // Escape, and nothing happened — the panel went on covering Crew's rail until
  // she found the toggle again. ⌘B had the same gap.
  //
  // Escape closes it however it was opened, from where the overlay's own focus
  // is: inside the panel, on its toggle, or nowhere (<body>). Focus then lands on
  // the toggle — the layout effect above moves it there from inside the panel,
  // and from <body> it is moved here. Three things are left alone, each on
  // purpose:
  //   - an Escape something else already answered (`defaultPrevented`): a menu,
  //     a popover or a dialog opened from a row closes first, and only the next
  //     Escape reaches the panel. Radix's layers listen in the capture phase and
  //     prevent the default when they dismiss, so a window listener in the bubble
  //     phase runs after them and can tell;
  //   - an Escape pressed on the page BEHIND the overlay: that control's own
  //     Escape (clear a field, cancel an edit) is what the person meant;
  //   - a docked column, which covers nothing and has nothing to dismiss.
  // A native listener, not a React `onKeyDown` on the panel: React bubbles a
  // portalled menu's keys through the row that opened it, and the toggle lives
  // outside the panel entirely.
  const overlayOpen = open && !isMobile && collapsible !== 'none';
  React.useEffect(() => {
    if (!overlayOpen) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented || event.isComposing) return;
      if (!sidebarIsOverlay()) return;
      const panel = panelRef.current;
      if (!panel) return;
      const target = event.target;
      const trigger =
        target instanceof Element ? target.closest<HTMLElement>('[data-sidebar="trigger"]') : null;
      const fromPanel = target instanceof Node && panel.contains(target);
      const fromNowhere =
        target === document.body || target === document.documentElement || target === document;
      if (!fromPanel && !trigger && !fromNowhere) return;
      event.preventDefault();
      setOpen(false);
      if (!fromPanel) {
        (trigger ?? document.querySelector<HTMLElement>('[data-sidebar="trigger"]'))?.focus();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [overlayOpen, setOpen]);

  if (collapsible === 'none') {
    return (
      <div
        data-slot="sidebar"
        className={cn(
          'bg-sidebar text-sidebar-foreground flex h-full w-(--sidebar-width) flex-col',
          className
        )}
        {...props}
      >
        {children}
      </div>
    );
  }

  if (isMobile) {
    return (
      <Sheet open={openMobile} onOpenChange={setOpenMobile} {...props}>
        <SheetContent
          data-sidebar="sidebar"
          data-slot="sidebar"
          data-mobile="true"
          className="bg-sidebar text-sidebar-foreground w-(--sidebar-width) min-w-(--sidebar-width) max-w-(--sidebar-width) px-4 py-0 [&>button]:hidden"
          style={
            {
              '--sidebar-width': SIDEBAR_WIDTH_MOBILE,
            } as React.CSSProperties
          }
          side={side}
        >
          <SheetHeader className="sr-only">
            <SheetTitle>Sidebar</SheetTitle>
            <SheetDescription>Displays the mobile sidebar.</SheetDescription>
          </SheetHeader>
          <div className="flex h-full w-full min-w-0 flex-col overflow-hidden">{children}</div>
        </SheetContent>
      </Sheet>
    );
  }

  const { onClick: onPanelClick, ...panelProps } = props;

  // AN OVERLAY STEPS ASIDE ONCE SOMETHING IS CHOSEN (triage T-66). Below rung 1
  // the open sidebar floats over the page, so leaving it open after a choice
  // kept it covering the very page the user just asked for — Crew's banner, the
  // start of the chat. A docked column is beside the content, not over it, and
  // stays exactly as the user left it.
  const handlePanelClick = (event: React.MouseEvent<HTMLDivElement>) => {
    onPanelClick?.(event);
    // `defaultPrevented` is deliberately NOT a veto: a client-side link cancels
    // the browser's navigation precisely because it is navigating.
    if (!open || !sidebarIsOverlay()) return;
    if (!isSidebarChoice(event.target, event.currentTarget)) return;
    const choice = {};
    choiceCloseRef.current = choice;
    window.setTimeout(() => {
      if (choiceCloseRef.current === choice) choiceCloseRef.current = null;
    }, 0);
    setOpen(false);
  };

  return (
    <div
      className="group peer text-sidebar-foreground hidden md:block"
      data-state={state}
      data-collapsible={state === 'collapsed' ? collapsible : ''}
      data-variant={variant}
      data-side={side}
      data-slot="sidebar"
    >
      {/* This is what handles the sidebar gap on desktop */}
      <div
        data-slot="sidebar-gap"
        className={cn(
          'relative w-(--sidebar-width) bg-transparent transition-[width] duration-[var(--motion-slow)] ease-[var(--ease-out)]',
          'group-data-[collapsible=offcanvas]:w-0',
          'group-data-[side=right]:rotate-180',
          variant === 'floating' || variant === 'inset'
            ? 'group-data-[collapsible=icon]:w-[calc(var(--sidebar-width-icon)+(--spacing(4)))]'
            : 'group-data-[collapsible=icon]:w-(--sidebar-width-icon)'
        )}
      />
      <div
        ref={panelRef}
        data-slot="sidebar-container"
        inert={offCanvas}
        onClick={handlePanelClick}
        className={cn(
          'biorouter-sidebar-shell bg-sidebar fixed inset-y-0 z-10 hidden h-svh w-(--sidebar-width) transition-transform duration-[var(--motion-slow)] ease-[var(--ease-out)] will-change-transform md:flex',
          side === 'left'
            ? 'left-0 group-data-[collapsible=offcanvas]:translate-x-[-100%]'
            : 'right-0 group-data-[collapsible=offcanvas]:translate-x-[100%]',
          // Adjust the padding for floating and inset variants.
          variant === 'floating' || variant === 'inset'
            ? 'py-2 pl-2 pr-4 group-data-[collapsible=icon]:w-[calc(var(--sidebar-width-icon)+(--spacing(4))+2px)]'
            : 'group-data-[collapsible=icon]:w-(--sidebar-width-icon) group-data-[side=left]:border-r group-data-[side=right]:border-l',
          className
        )}
        {...panelProps}
      >
        <div
          data-sidebar="sidebar"
          data-slot="sidebar-inner"
          className="bg-sidebar flex h-full w-full flex-col overflow-hidden group-data-[variant=floating]:rounded-container group-data-[variant=floating]:border"
        >
          {children}
        </div>
        <SidebarResizeHandle />
      </div>
    </div>
  );
}

/**
 * The sidebar's drag edge.
 *
 * WHY IT SITS INSIDE THE SIDEBAR'S OWN BOX. `sidebar-container` is `z-10` and
 * the route's `<main>` inside `SidebarInset` is `z-[60]`, so any part of this
 * handle that hung past the sidebar's right edge would be painted under the
 * content pane and silently un-grabbable — a control that looks present and
 * does nothing. The 8px target therefore sits wholly within the sidebar, flush
 * to the edge.
 *
 * WHY THE STYLING IS AUTHORED CSS. The hover hairline and `cursor: col-resize`
 * are the only affordance the control has, and a Tailwind utility can silently
 * fail to generate when the renderer's watcher is off (the composer's focus edge
 * hit exactly this — see `styles/composerFocus.test.ts`). An invisible edge with
 * a default cursor is indistinguishable from a missing feature, so the rule is
 * written out in `main.css`, which also hides the handle while the sidebar is
 * collapsed.
 *
 * It is a focusable `role="separator"` — the ARIA window-splitter pattern — so
 * the width is reachable without a pointer, and reports its bounds so a screen
 * reader can say where the edge currently is.
 */
function SidebarResizeHandle() {
  const { width, isResizing, startResize, nudgeWidth, resetWidth } = useSidebar();

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowLeft') {
      event.preventDefault();
      nudgeWidth(-SIDEBAR_WIDTH_KEYBOARD_STEP);
    } else if (event.key === 'ArrowRight') {
      event.preventDefault();
      nudgeWidth(SIDEBAR_WIDTH_KEYBOARD_STEP);
    } else if (event.key === 'Home') {
      event.preventDefault();
      nudgeWidth(SIDEBAR_MIN_WIDTH - width);
    } else if (event.key === 'End') {
      event.preventDefault();
      nudgeWidth(SIDEBAR_MAX_WIDTH - width);
    } else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      resetWidth();
    }
  };

  return (
    <div
      data-slot="sidebar-resize-handle"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize sidebar"
      aria-valuenow={width}
      aria-valuemin={SIDEBAR_MIN_WIDTH}
      aria-valuemax={SIDEBAR_MAX_WIDTH}
      aria-valuetext={`${width} pixels`}
      data-resizing={isResizing ? 'true' : undefined}
      tabIndex={0}
      title="Drag to resize · double-click to reset"
      onPointerDown={startResize}
      onDoubleClick={resetWidth}
      onKeyDown={handleKeyDown}
      className="biorouter-sidebar-resize-handle"
    />
  );
}

function SidebarTrigger({
  className,
  onClick,
  size = 'sm',
  ...props
}: React.ComponentProps<typeof Button>) {
  const { toggleSidebar, isMobile, open, openMobile } = useSidebar();

  return (
    <Button
      data-sidebar="trigger"
      data-slot="sidebar-trigger"
      variant="ghost"
      size={size}
      className={cn(className)}
      // A disclosure: it shows and hides the sidebar, so it says which (Q2-51).
      aria-expanded={isMobile ? openMobile : open}
      onClick={(event) => {
        onClick?.(event);
        toggleSidebar();
      }}
      {...props}
    >
      <PanelLeftIcon />
      <span className="sr-only">Toggle sidebar</span>
    </Button>
  );
}

function SidebarRail({ className, ...props }: React.ComponentProps<'button'>) {
  const { toggleSidebar, isMobile, open, openMobile } = useSidebar();

  return (
    <button
      data-sidebar="rail"
      data-slot="sidebar-rail"
      aria-label="Toggle sidebar"
      aria-expanded={isMobile ? openMobile : open}
      tabIndex={-1}
      onClick={toggleSidebar}
      title="Toggle sidebar"
      className={cn(
        'hover:after:bg-sidebar-border absolute inset-y-0 z-20 hidden w-4 -translate-x-1/2 transition-all ease-linear group-data-[side=left]:-right-4 group-data-[side=right]:left-0 after:absolute after:inset-y-0 after:left-1/2 after:w-[2px] sm:flex',
        'in-data-[side=left]:cursor-w-resize in-data-[side=right]:cursor-e-resize',
        '[[data-side=left][data-state=collapsed]_&]:cursor-e-resize [[data-side=right][data-state=collapsed]_&]:cursor-w-resize',
        'hover:group-data-[collapsible=offcanvas]:bg-sidebar group-data-[collapsible=offcanvas]:translate-x-0 group-data-[collapsible=offcanvas]:after:left-full',
        '[[data-side=left][data-collapsible=offcanvas]_&]:-right-2',
        '[[data-side=right][data-collapsible=offcanvas]_&]:-left-2',
        className
      )}
      {...props}
    />
  );
}

/**
 * The content column beside the sidebar. A `<div>`, deliberately NOT a `<main>`
 * (triage T-61): the app's one main landmark is the route container that
 * `AppLayout` renders INSIDE this box, and shadcn's `<main>` here nested a
 * second one around it, so a screen reader's landmark list offered "main" twice
 * for one page. `sidebar.test.tsx` renders the real layout and counts them.
 */
function SidebarInset({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="sidebar-inset"
      className={cn(
        'biorouter-sidebar-inset-depth bg-background relative flex w-full flex-1 flex-col min-w-0',
        // For inset variant (used in the app): flush against the straight,
        // full-height sidebar — no rounded corners or floating shadow, which
        // otherwise leave a notch where the rounded panel meets the square sidebar.
        'md:peer-data-[variant=inset]:ml-0',
        // For offcanvas variant - ensure content doesn't go under sidebar
        'md:peer-data-[collapsible=offcanvas]:peer-data-[state=expanded]:ml-[var(--sidebar-width)]',
        'md:peer-data-[collapsible=offcanvas]:peer-data-[state=collapsed]:ml-0',
        // Smooth transition when sidebar state changes
        'transition-[margin-left] duration-[var(--motion-slow)] ease-[var(--ease-out)]',
        className
      )}
      {...props}
    />
  );
}

function SidebarInput({ className, ...props }: React.ComponentProps<typeof Input>) {
  return (
    <Input
      data-slot="sidebar-input"
      data-sidebar="input"
      className={cn('bg-background h-8 w-full shadow-none', className)}
      {...props}
    />
  );
}

function SidebarHeader({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="sidebar-header"
      data-sidebar="header"
      className={cn('flex flex-col gap-2 px-2 pt-1', className)}
      {...props}
    />
  );
}

function SidebarFooter({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="sidebar-footer"
      data-sidebar="footer"
      className={cn('flex flex-col gap-2 p-2', className)}
      {...props}
    />
  );
}

function SidebarSeparator({ className, ...props }: React.ComponentProps<typeof Separator>) {
  return (
    <Separator
      data-slot="sidebar-separator"
      data-sidebar="separator"
      className={cn('bg-border-strong ml-5 my-2 !w-8', className)}
      {...props}
    />
  );
}

function SidebarContent({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="sidebar-content"
      data-sidebar="content"
      className={cn(
        'flex min-h-0 w-full min-w-0 flex-1 flex-col gap-2 overflow-y-auto overflow-x-hidden group-data-[collapsible=icon]:overflow-hidden',
        className
      )}
      {...props}
    />
  );
}

function SidebarGroup({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="sidebar-group"
      data-sidebar="group"
      className={cn('relative flex w-full min-w-0 flex-col px-2', className)}
      {...props}
    />
  );
}

function SidebarGroupLabel({
  className,
  asChild = false,
  ...props
}: React.ComponentProps<'div'> & { asChild?: boolean }) {
  const Comp = asChild ? Slot : 'div';

  return (
    <Comp
      data-slot="sidebar-group-label"
      data-sidebar="group-label"
      className={cn(
        'text-sidebar-foreground/70 ring-sidebar-ring flex h-8 shrink-0 items-center rounded-element px-2 text-supporting transition-[margin,opacity] duration-200 ease-linear [&>svg]:size-4 [&>svg]:shrink-0',
        'group-data-[collapsible=icon]:-mt-8 group-data-[collapsible=icon]:opacity-0',
        className
      )}
      {...props}
    />
  );
}

function SidebarGroupAction({
  className,
  asChild = false,
  ...props
}: React.ComponentProps<'button'> & { asChild?: boolean }) {
  const Comp = asChild ? Slot : 'button';

  return (
    <Comp
      data-slot="sidebar-group-action"
      data-sidebar="group-action"
      className={cn(
        'text-sidebar-foreground ring-sidebar-ring hover:bg-sidebar-accent hover:text-sidebar-accent-foreground absolute top-3.5 right-3 flex aspect-square w-5 items-center justify-center rounded-element p-0 transition-transform [&>svg]:size-4 [&>svg]:shrink-0',
        // Increases the hit area of the button on mobile.
        'after:absolute after:-inset-2 md:after:hidden',
        'group-data-[collapsible=icon]:hidden',
        className
      )}
      {...props}
    />
  );
}

function SidebarGroupContent({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="sidebar-group-content"
      data-sidebar="group-content"
      className={cn('w-full text-body', className)}
      {...props}
    />
  );
}

function SidebarMenu({ className, ...props }: React.ComponentProps<'ul'>) {
  return (
    <ul
      data-slot="sidebar-menu"
      data-sidebar="menu"
      className={cn('flex w-full min-w-0 flex-col gap-1', className)}
      {...props}
    />
  );
}

function SidebarMenuItem({ className, ...props }: React.ComponentProps<'li'>) {
  return (
    <li
      data-slot="sidebar-menu-item"
      data-sidebar="menu-item"
      className={cn('group/menu-item relative', className)}
      {...props}
    />
  );
}

const sidebarMenuButtonVariants = cva(
  'peer/menu-button flex w-full items-center gap-2 overflow-hidden rounded-element px-2 py-0 text-left text-label ring-sidebar-ring transition-[width,height,padding] hover:bg-sidebar-accent hover:text-sidebar-accent-foreground active:bg-sidebar-accent active:text-sidebar-accent-foreground disabled:pointer-events-none disabled:opacity-50 group-has-data-[sidebar=menu-action]/menu-item:pr-8 aria-disabled:pointer-events-none aria-disabled:opacity-50 data-[active=true]:bg-sidebar-accent data-[active=true]:data-[active=true]:text-sidebar-accent-foreground data-[state=open]:hover:bg-sidebar-accent data-[state=open]:hover:text-sidebar-accent-foreground [&>span:last-child]:truncate [&>svg]:size-4 [&>svg]:shrink-0',
  {
    variants: {
      variant: {
        default: 'hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
        outline: 'bg-background hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
      },
      size: {
        default: 'h-control-md text-label',
        sm: 'h-control-sm text-supporting',
        lg: 'h-12 text-label group-data-[collapsible=icon]:p-0!',
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'default',
    },
  }
);

function SidebarMenuButton({
  asChild = false,
  isActive = false,
  variant = 'default',
  size = 'default',
  tooltip,
  className,
  onClick,
  ...props
}: React.ComponentProps<'button'> & {
  asChild?: boolean;
  isActive?: boolean;
  tooltip?: string | React.ComponentProps<typeof TooltipContent>;
} & VariantProps<typeof sidebarMenuButtonVariants>) {
  const Comp = asChild ? Slot : 'button';
  const { isMobile, state, setOpenMobile } = useSidebar();

  const handleClick = React.useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      // Call the original onClick handler if provided
      onClick?.(event);

      // Auto-close mobile sidebar when menu item is clicked
      if (isMobile) {
        setOpenMobile(false);
      }
    },
    [onClick, isMobile, setOpenMobile]
  );

  const button = (
    <Comp
      data-slot="sidebar-menu-button"
      data-sidebar="menu-button"
      data-size={size}
      data-active={isActive}
      className={cn(sidebarMenuButtonVariants({ variant, size }), className)}
      onClick={handleClick}
      {...props}
    />
  );

  if (!tooltip) {
    return button;
  }

  if (typeof tooltip === 'string') {
    tooltip = {
      children: tooltip,
    };
  }

  // A row's tooltip is for the collapsed icon rail, where the row shows no label. Anywhere else
  // it could never be seen, so it must never OPEN (live QA round 4, Q4-53). Drawing it `hidden`
  // was not enough: a hidden tooltip still opens on focus, and an open tooltip is a dismissable
  // layer. After Erin tabbed onto Home in the overlay, that invisible layer took her first Escape
  // and the overlay only closed on the second.
  //
  // The row keeps its Tooltip wrapper and holds it shut, rather than dropping the wrapper while
  // expanded: a wrapper that comes and goes changes the element type at this position, so React
  // would remount the button on every expand and collapse, and a row holding focus (the Escape
  // that closes the overlay is pressed on one) would drop that focus on <body>.
  // `sidebar.test.tsx` pins both halves.
  const tooltipHidden = state !== 'collapsed' || isMobile;

  return (
    <Tooltip open={tooltipHidden ? false : undefined}>
      <TooltipTrigger asChild>{button}</TooltipTrigger>
      <TooltipContent side="right" align="center" hidden={tooltipHidden} {...tooltip} />
    </Tooltip>
  );
}

function SidebarMenuAction({
  className,
  asChild = false,
  showOnHover = false,
  ...props
}: React.ComponentProps<'button'> & {
  asChild?: boolean;
  showOnHover?: boolean;
}) {
  const Comp = asChild ? Slot : 'button';

  return (
    <Comp
      data-slot="sidebar-menu-action"
      data-sidebar="menu-action"
      className={cn(
        'text-sidebar-foreground ring-sidebar-ring hover:bg-sidebar-accent hover:text-sidebar-accent-foreground peer-hover/menu-button:text-sidebar-accent-foreground absolute top-1.5 right-1 flex aspect-square w-5 items-center justify-center rounded-element p-0 transition-transform [&>svg]:size-4 [&>svg]:shrink-0',
        // Increases the hit area of the button on mobile.
        'after:absolute after:-inset-2 md:after:hidden',
        'peer-data-[size=sm]/menu-button:top-1',
        'peer-data-[size=default]/menu-button:top-1.5',
        'peer-data-[size=lg]/menu-button:top-2.5',
        'group-data-[collapsible=icon]:hidden',
        showOnHover &&
          'peer-data-[active=true]/menu-button:text-sidebar-accent-foreground group-focus-within/menu-item:opacity-100 group-hover/menu-item:opacity-100 data-[state=open]:opacity-100 md:opacity-0',
        className
      )}
      {...props}
    />
  );
}

function SidebarMenuBadge({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="sidebar-menu-badge"
      data-sidebar="menu-badge"
      className={cn(
        'text-sidebar-foreground pointer-events-none absolute right-1 flex h-5 min-w-5 items-center justify-center rounded-inner px-1 text-supporting tabular-nums select-none',
        'peer-hover/menu-button:text-sidebar-accent-foreground peer-data-[active=true]/menu-button:text-sidebar-accent-foreground',
        'peer-data-[size=sm]/menu-button:top-1',
        'peer-data-[size=default]/menu-button:top-1.5',
        'peer-data-[size=lg]/menu-button:top-2.5',
        'group-data-[collapsible=icon]:hidden',
        className
      )}
      {...props}
    />
  );
}

function SidebarMenuSkeleton({
  className,
  showIcon = false,
  ...props
}: React.ComponentProps<'div'> & {
  showIcon?: boolean;
}) {
  // Random width between 50 to 90%.
  const width = React.useMemo(() => {
    return `${Math.floor(Math.random() * 40) + 50}%`;
  }, []);

  return (
    <div
      data-slot="sidebar-menu-skeleton"
      data-sidebar="menu-skeleton"
      className={cn('flex h-8 items-center gap-2 rounded-element px-2', className)}
      {...props}
    >
      {showIcon && (
        <Skeleton className="size-4 rounded-element" data-sidebar="menu-skeleton-icon" />
      )}
      <Skeleton
        className="h-4 max-w-(--skeleton-width) flex-1"
        data-sidebar="menu-skeleton-text"
        style={
          {
            '--skeleton-width': width,
          } as React.CSSProperties
        }
      />
    </div>
  );
}

function SidebarMenuSub({ className, ...props }: React.ComponentProps<'ul'>) {
  return (
    <ul
      data-slot="sidebar-menu-sub"
      data-sidebar="menu-sub"
      className={cn(
        'border-sidebar-border mx-3.5 flex min-w-0 translate-x-px flex-col gap-1 border-l px-2.5 py-0.5',
        'group-data-[collapsible=icon]:hidden',
        className
      )}
      {...props}
    />
  );
}

function SidebarMenuSubItem({ className, ...props }: React.ComponentProps<'li'>) {
  return (
    <li
      data-slot="sidebar-menu-sub-item"
      data-sidebar="menu-sub-item"
      className={cn('group/menu-sub-item relative', className)}
      {...props}
    />
  );
}

function SidebarMenuSubButton({
  asChild = false,
  size = 'md',
  isActive = false,
  className,
  ...props
}: React.ComponentProps<'a'> & {
  asChild?: boolean;
  size?: 'sm' | 'md';
  isActive?: boolean;
}) {
  const Comp = asChild ? Slot : 'a';

  return (
    <Comp
      data-slot="sidebar-menu-sub-button"
      data-sidebar="menu-sub-button"
      data-size={size}
      data-active={isActive}
      className={cn(
        'text-sidebar-foreground ring-sidebar-ring hover:bg-sidebar-accent hover:text-sidebar-accent-foreground active:bg-sidebar-accent active:text-sidebar-accent-foreground [&>svg]:text-sidebar-accent-foreground flex h-7 min-w-0 -translate-x-px items-center gap-2 overflow-hidden rounded-element px-2 disabled:pointer-events-none disabled:opacity-50 aria-disabled:pointer-events-none aria-disabled:opacity-50 [&>span:last-child]:truncate [&>svg]:size-4 [&>svg]:shrink-0',
        'data-[active=true]:bg-sidebar-accent data-[active=true]:text-sidebar-accent-foreground',
        size === 'sm' && 'text-supporting',
        size === 'md' && 'text-label',
        'group-data-[collapsible=icon]:hidden',
        className
      )}
      {...props}
    />
  );
}

export {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupAction,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInput,
  SidebarInset,
  SidebarMenu,
  SidebarMenuAction,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSkeleton,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  SidebarProvider,
  SidebarRail,
  SidebarSeparator,
  SidebarTrigger,
  useSidebar,
};
