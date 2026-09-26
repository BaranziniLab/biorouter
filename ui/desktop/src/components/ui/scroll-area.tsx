import * as React from 'react';
import * as ScrollAreaPrimitive from '@radix-ui/react-scroll-area';

type ScrollBehavior = 'auto' | 'smooth';

import { cn } from '../../utils';

export interface ScrollAreaHandle {
  scrollToBottom: (behavior?: ScrollBehavior) => void;
  scrollToPosition: (options: { top: number; behavior?: ScrollBehavior }) => void;
  isAtBottom: () => boolean;
  isFollowing: boolean;
  viewportRef: React.RefObject<HTMLDivElement | null>;
}

interface ScrollAreaProps extends React.ComponentPropsWithoutRef<typeof ScrollAreaPrimitive.Root> {
  autoScroll?: boolean;
  onScrollChange?: (isAtBottom: boolean) => void;
  /* padding needs to be passed into the container inside ScrollArea to avoid pushing the scrollbar out */
  paddingX?: number;
  paddingY?: number;
  handleScroll?: (viewport: HTMLDivElement) => void;
  /**
   * Keep the transcript's BOTTOM edge in place when its viewport changes height,
   * while this returns true (and on the one resize after it stops returning true).
   *
   * A chat reads bottom-up: the newest message sits against the composer. A
   * stacked artifact preview (rung 2 of the yield ladder) opens ABOVE the
   * transcript and takes its height from the top, and a plain viewport keeps
   * `scrollTop` — so the newest lines slid under the composer the moment the
   * preview opened. A function rather than a boolean because it is asked at the
   * instant of the resize, which lands between React commits.
   *
   * Opt-in, and only the live chat's transcript opts in. Every other scroll area,
   * and the live chat itself whenever no stacked sheet is on screen, keeps the
   * behaviour it had.
   */
  anchorBottomOnResize?: () => boolean;
}

const ScrollArea = React.forwardRef<ScrollAreaHandle, ScrollAreaProps>(
  (
    {
      className,
      children,
      autoScroll = false,
      onScrollChange,
      paddingX,
      paddingY,
      handleScroll: handleScrollProp,
      anchorBottomOnResize,
      ...props
    },
    ref
  ) => {
    const rootRef = React.useRef<React.ElementRef<typeof ScrollAreaPrimitive.Root>>(null);
    const viewportRef = React.useRef<HTMLDivElement>(null);
    const viewportEndRef = React.useRef<HTMLDivElement>(null);
    const [isFollowing, setIsFollowing] = React.useState(true);
    const [isScrolled, setIsScrolled] = React.useState(false);
    const userScrolledUpRef = React.useRef(false);
    const lastScrollHeightRef = React.useRef(0);
    const isActivelyScrollingRef = React.useRef(false);
    const scrollTimeoutRef = React.useRef<number | null>(null);

    const BOTTOM_SCROLL_THRESHOLD = 200;

    const isAtBottom = React.useCallback(() => {
      if (!viewportRef.current) return false;

      const viewport = viewportRef.current;
      const { scrollHeight, scrollTop, clientHeight } = viewport;
      const distanceFromBottom = scrollHeight - scrollTop - clientHeight;

      return distanceFromBottom <= BOTTOM_SCROLL_THRESHOLD;
    }, []);

    // The bottom edge the resize anchor (below) keeps in place, in content
    // coordinates; null while no anchor is installed.
    const anchorBottomRef = React.useRef<number | null>(null);

    const scrollToBottom = React.useCallback(
      (behavior: ScrollBehavior = 'smooth') => {
        if (viewportRef.current) {
          // An explicit scroll to the bottom is where the reader's bottom edge now
          // is, even when it lands mid-reflow: the anchor's scroll listener skips
          // a scroll that arrives before the resize observer has seen a new
          // height, and would otherwise put the viewport back at the bottom edge
          // it remembered before this scroll (measured: a channel opening while
          // a note above its composer grew landed 12px from the TOP).
          if (anchorBottomRef.current !== null)
            anchorBottomRef.current = viewportRef.current.scrollHeight;
          viewportRef.current.scrollTo({
            top: viewportRef.current.scrollHeight,
            behavior,
          });
          // When explicitly scrolling to bottom, reset the following state
          setIsFollowing(true);
          userScrolledUpRef.current = false;
          onScrollChange?.(true);
        }
      },
      [onScrollChange]
    );

    const scrollToPosition = React.useCallback(
      ({ top, behavior = 'smooth' }: { top: number; behavior?: ScrollBehavior }) => {
        if (viewportRef.current) {
          viewportRef.current.scrollTo({
            top,
            behavior,
          });
        }
      },
      []
    );

    // Expose the scroll methods to parent components
    React.useImperativeHandle(
      ref,
      () => ({
        scrollToBottom,
        scrollToPosition,
        isAtBottom,
        isFollowing,
        viewportRef,
      }),
      [scrollToBottom, scrollToPosition, isAtBottom, isFollowing]
    );

    // track last scroll position to detect user-initiated scrolling
    const lastScrollTopRef = React.useRef(0);
    // The predicate, read at resize time, and the viewport height the anchor last
    // saw. A scroll event that arrives while the height differs from this one is
    // the LAYOUT moving, not the reader (see handleScroll).
    const anchorPredicateRef = React.useRef(anchorBottomOnResize);
    anchorPredicateRef.current = anchorBottomOnResize;
    const anchorEngagedRef = React.useRef(false);
    const anchoredHeightRef = React.useRef<number | null>(null);

    // Handle scroll events to update isFollowing state
    const handleScroll = React.useCallback(() => {
      if (!viewportRef.current) return;

      const viewport = viewportRef.current;

      // ⚠ A scroll that arrives WITH a viewport resize is layout, not the reader.
      // When a stacked preview opens, Chromium's scroll anchoring can adjust
      // `scrollTop` during the reflow and dispatch this event BEFORE the resize
      // observer below has run. Read as a reader's scroll, it turned following
      // OFF and left the transcript hundreds of pixels above its newest line for
      // as long as the preview stayed open (measured on a prototype). While the
      // anchor is engaged the observer owns this change, so it is not a scroll.
      if (
        (anchorEngagedRef.current || anchorPredicateRef.current?.()) &&
        anchoredHeightRef.current !== null &&
        viewport.clientHeight !== anchoredHeightRef.current
      ) {
        lastScrollTopRef.current = viewport.scrollTop;
        return;
      }

      const { scrollTop } = viewport;
      const currentIsAtBottom = isAtBottom();

      // detect if this is a user-initiated scroll (position changed from last known position)
      const scrollDelta = Math.abs(scrollTop - lastScrollTopRef.current);
      if (scrollDelta > 0) {
        // Mark that user is actively scrolling immediately
        isActivelyScrollingRef.current = true;

        // clear any existing timeout and set a new one
        if (scrollTimeoutRef.current) {
          clearTimeout(scrollTimeoutRef.current);
        }

        // mark as not actively scrolling
        scrollTimeoutRef.current = window.setTimeout(() => {
          isActivelyScrollingRef.current = false;
        }, 100);
      }

      lastScrollTopRef.current = scrollTop;

      // Detect if user manually scrolled up from the bottom
      if (!currentIsAtBottom && isFollowing) {
        // user scrolled up, disabling auto-scroll
        userScrolledUpRef.current = true;
        setIsFollowing(false);
        onScrollChange?.(false);
      } else if (currentIsAtBottom && userScrolledUpRef.current) {
        // user scrolled back to bottom
        userScrolledUpRef.current = false;
        setIsFollowing(true);
        onScrollChange?.(true);
      }

      setIsScrolled(scrollTop > 0);

      if (handleScrollProp) {
        handleScrollProp(viewport);
      }
    }, [isAtBottom, isFollowing, onScrollChange, handleScrollProp]);

    // Auto-scroll when content changes and user is following
    React.useEffect(() => {
      if (!autoScroll || !viewportRef.current) return;

      const viewport = viewportRef.current;
      const currentScrollHeight = viewport.scrollHeight;

      // Only auto-scroll if:
      // 1. Content has actually grown (new content added)
      // 2. User was following (at the bottom)
      // 3. User hasn't manually scrolled up
      // 4. User is not actively scrolling
      if (
        currentScrollHeight > lastScrollHeightRef.current &&
        isFollowing &&
        !userScrolledUpRef.current &&
        !isActivelyScrollingRef.current
      ) {
        // Use requestAnimationFrame to ensure DOM has updated.
        // Follow streaming content with an INSTANT scroll: a smooth scroll is
        // re-issued on every content chunk and never settles, so it fights the
        // growing transcript and janks. Smooth is reserved for the explicit
        // jump-to-bottom button (scrollToBottom).
        requestAnimationFrame(() => {
          if (viewportRef.current && !isActivelyScrollingRef.current) {
            viewportRef.current.scrollTo({
              top: viewportRef.current.scrollHeight,
              behavior: 'auto',
            });
          }
        });
      }

      lastScrollHeightRef.current = currentScrollHeight;
    }, [children, autoScroll, isFollowing]);

    // THE BOTTOM ANCHOR (`anchorBottomOnResize`). Remembers where the viewport's
    // bottom edge sits in content coordinates, and when the viewport changes
    // height writes `scrollTop = bottom − height`, so the line against the
    // composer stays against the composer.
    //
    // ⚠ ABSOLUTE, NOT A DELTA. A prototype added `oldHeight − newHeight` to
    // `scrollTop`, and closing the preview then scrolled the transcript to the TOP:
    // a viewport that GROWS past its content has `scrollTop` clamped by the browser
    // during layout, before this callback runs, so the delta was applied twice
    // (measured: 480 → clamp 480 → minus 478 = 2). Writing the remembered bottom
    // edge is idempotent under the clamp.
    //
    // It stays engaged for ONE resize after the predicate goes false, because the
    // resize that ends a stacked sheet — closing it, or the pane widening it back
    // into a side column — is the commit that also turns the predicate off.
    const hasBottomAnchor = Boolean(anchorBottomOnResize);
    React.useEffect(() => {
      const viewport = viewportRef.current;
      if (!autoScroll || !hasBottomAnchor || !viewport) return;
      if (typeof ResizeObserver === 'undefined') return;
      let lastHeight = viewport.clientHeight;
      anchorBottomRef.current = viewport.scrollTop + lastHeight;
      anchoredHeightRef.current = lastHeight;
      const remember = () => {
        // Mid-reflow the height is already new and the observer has not run: the
        // bottom edge it would record is the one being moved, not the reader's.
        if (viewport.clientHeight !== lastHeight) return;
        anchorBottomRef.current = viewport.scrollTop + viewport.clientHeight;
      };
      const observer = new ResizeObserver(() => {
        const height = viewport.clientHeight;
        if (height === lastHeight) return;
        const engaged = anchorPredicateRef.current?.() ?? false;
        const anchor = engaged || anchorEngagedRef.current;
        anchorEngagedRef.current = engaged;
        lastHeight = height;
        anchoredHeightRef.current = height;
        if (anchor) {
          const bottom = anchorBottomRef.current ?? viewport.scrollTop + height;
          viewport.scrollTop = Math.max(0, bottom - height);
          lastScrollTopRef.current = viewport.scrollTop;
        }
        anchorBottomRef.current = viewport.scrollTop + height;
      });
      viewport.addEventListener('scroll', remember, { passive: true });
      observer.observe(viewport);
      return () => {
        observer.disconnect();
        viewport.removeEventListener('scroll', remember);
        anchorBottomRef.current = null;
        anchoredHeightRef.current = null;
        anchorEngagedRef.current = false;
      };
      // `anchorBottomOnResize` is read through the ref; only its presence matters here.
    }, [autoScroll, hasBottomAnchor]);

    // Add scroll event listener
    React.useEffect(() => {
      const viewport = viewportRef.current;
      if (!viewport) return;

      viewport.addEventListener('scroll', handleScroll, { passive: true });
      return () => {
        viewport.removeEventListener('scroll', handleScroll);
        if (scrollTimeoutRef.current) {
          clearTimeout(scrollTimeoutRef.current);
        }
      };
    }, [handleScroll]);

    return (
      <ScrollAreaPrimitive.Root
        ref={rootRef}
        className={cn('relative overflow-hidden', className)}
        data-scrolled={isScrolled}
        {...props}
      >
        <div
          className={cn(
            'absolute top-0 left-0 right-0 z-10 transition-opacity duration-[var(--motion-base)]'
          )}
        />
        <ScrollAreaPrimitive.Viewport
          ref={viewportRef}
          className="h-full w-full rounded-[inherit] [&>div]:!block"
        >
          <div className={cn(paddingX ? `px-${paddingX}` : '', paddingY ? `py-${paddingY}` : '')}>
            {children}
            {autoScroll && <div ref={viewportEndRef} style={{ height: '1px' }} />}
          </div>
        </ScrollAreaPrimitive.Viewport>
        <ScrollBar />
        <ScrollAreaPrimitive.Corner />
      </ScrollAreaPrimitive.Root>
    );
  }
);
ScrollArea.displayName = ScrollAreaPrimitive.Root.displayName;

const ScrollBar = React.forwardRef<
  React.ElementRef<typeof ScrollAreaPrimitive.ScrollAreaScrollbar>,
  React.ComponentPropsWithoutRef<typeof ScrollAreaPrimitive.ScrollAreaScrollbar>
>(({ className, orientation = 'vertical', ...props }, ref) => (
  <ScrollAreaPrimitive.ScrollAreaScrollbar
    ref={ref}
    orientation={orientation}
    className={cn(
      'flex touch-none select-none transition-colors',
      orientation === 'vertical' && 'h-full w-2.5 border-l border-l-transparent p-[1px]',
      orientation === 'horizontal' && 'h-2.5 flex-col border-t border-t-transparent p-[1px]',
      className
    )}
    {...props}
  >
    <ScrollAreaPrimitive.ScrollAreaThumb className="relative flex-1 rounded-full bg-border dark:bg-border-dark" />
  </ScrollAreaPrimitive.ScrollAreaScrollbar>
));
ScrollBar.displayName = ScrollAreaPrimitive.ScrollAreaScrollbar.displayName;

export { ScrollArea, ScrollBar };
