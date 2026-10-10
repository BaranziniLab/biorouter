'use client';
import * as React from 'react';

import { cn } from '../../utils';
import * as TabsPrimitive from '@radix-ui/react-tabs';

const Tabs = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Root>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Root
    ref={ref}
    className={cn('flex flex-col rounded-element text-text-muted', className)}
    {...props}
  />
));
Tabs.displayName = TabsPrimitive.Root.displayName;

interface IndicatorPlacement {
  x: number;
  width: number;
}

/**
 * Where the active trigger's underline belongs, measured against the list. `null` until a real
 * measurement exists: jsdom, a hidden ancestor and the first paint all measure 0, and then each
 * trigger's own `::after` bar is the underline, so nothing slides in from x=0.
 *
 * The underline glides only when the PERSON picks another tab (principle 9). A tab that moved
 * because the layout did (a resize, a tab added before it, a label changing) is measured by the
 * observers and lands at rest, and `data-motion-layout` stops the glide while the window or the
 * sidebar is being dragged.
 */
function useTabsIndicator(list: React.RefObject<HTMLDivElement | null>) {
  const [placement, setPlacement] = React.useState<IndicatorPlacement | null>(null);
  const placementRef = React.useRef<IndicatorPlacement | null>(null);
  // The first placement lands at rest (principle 9); only the person's later picks travel.
  const [still, setStill] = React.useState(true);

  React.useLayoutEffect(() => {
    const node = list.current;
    if (!node) return;

    const measure = (fromLayout: boolean) => {
      const active = node.querySelector<HTMLElement>('[role="tab"][data-state="active"]');
      const place = (next: IndicatorPlacement | null) => {
        const current = placementRef.current;
        if (
          current === next ||
          (current && next && current.x === next.x && current.width === next.width)
        ) {
          return;
        }
        placementRef.current = next;
        if (fromLayout && current && next) setStill(true);
        setPlacement(next);
      };
      if (!active) {
        place(null);
        return;
      }
      const listBox = node.getBoundingClientRect();
      const tabBox = active.getBoundingClientRect();
      if (tabBox.width <= 0 || listBox.width <= 0) {
        place(null);
        return;
      }
      // An ancestor may be mid-animation (a dialog pops in at scale .98): both boxes are then
      // scaled, while the indicator's `translate` and `width` are in the list's own pixels. The
      // list's layout width tells the scale; a deviation inside the rounding of `offsetWidth` is
      // none, so an unscaled list measures exactly.
      const layoutWidth = node.offsetWidth;
      const ratio = layoutWidth > 0 ? listBox.width / layoutWidth : 1;
      const scale = Math.abs(ratio - 1) <= 0.5 / Math.max(layoutWidth, 1) + 0.002 ? 1 : ratio;
      place({
        // Absolute boxes are placed from the padding edge, so the list's own start border
        // (Knowledge draws one) comes off.
        x: (tabBox.left - listBox.left) / scale - node.clientLeft + node.scrollLeft,
        width: tabBox.width / scale,
      });
    };

    measure(false);

    const resizes =
      typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => measure(true));
    const observed = new Set<Element>();
    const observeTabs = () => {
      if (!resizes) return;
      node.querySelectorAll('[role="tab"]').forEach((tab) => {
        if (observed.has(tab)) return;
        observed.add(tab);
        resizes.observe(tab);
      });
    };
    resizes?.observe(node);
    observeTabs();

    const mutations = new MutationObserver((records) => {
      // A tab added or removed moves the others: that is the layout's doing, not a pick. A
      // `data-state` flip is the pick (or the controlled value changing under it).
      const structural = records.some((record) => record.type === 'childList');
      if (structural) observeTabs();
      measure(structural);
    });
    mutations.observe(node, {
      attributes: true,
      attributeFilter: ['data-state'],
      childList: true,
      subtree: true,
    });

    return () => {
      mutations.disconnect();
      resizes?.disconnect();
    };
  }, [list]);

  React.useEffect(() => {
    if (!placement || !still) return;
    // One frame at rest, then later picks animate.
    const frame = requestAnimationFrame(() => setStill(false));
    return () => cancelAnimationFrame(frame);
  }, [placement, still]);

  return { placement, still };
}

/**
 * The tab list. One underline travels under the active trigger (`translate` and `width` over
 * `--dur-fast-max`, a named exception in principle 9: it is absolutely positioned and moves no
 * other box). Its first placement does not animate. Authored in main.css (`.br-tabs-*`).
 */
const TabsList = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.List>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.List>
>(({ className, children, ...props }, forwardedRef) => {
  const listRef = React.useRef<HTMLDivElement | null>(null);
  const setRefs = React.useCallback(
    (node: HTMLDivElement | null) => {
      listRef.current = node;
      if (typeof forwardedRef === 'function') forwardedRef(node);
      else if (forwardedRef) forwardedRef.current = node;
    },
    [forwardedRef]
  );
  const { placement, still } = useTabsIndicator(listRef);

  return (
    <TabsPrimitive.List
      ref={setRefs}
      data-placed={placement ? 'true' : 'false'}
      className={cn(
        'br-tabs-list flex items-center gap-5 border-b border-border-subtle',
        className
      )}
      {...props}
    >
      {children}
      <span
        aria-hidden
        className="br-tabs-indicator"
        data-placed={placement ? 'true' : 'false'}
        data-motion-layout=""
        data-motion-still={still ? '' : undefined}
        style={
          placement
            ? ({
                width: `${placement.width}px`,
                translate: `${placement.x}px 0`,
              } as React.CSSProperties)
            : undefined
        }
      />
    </TabsPrimitive.List>
  );
});

TabsList.displayName = TabsPrimitive.List.displayName;

const TabsTrigger = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Trigger
    ref={ref}
    className={cn(
      // The `after:` bar is each trigger's own underline until the list's indicator is placed
      // (and the hook the focus rule firms to 3px); main.css hides it once the indicator exists.
      "relative inline-flex items-center h-control-lg whitespace-nowrap text-label text-text-muted transition-colors hover:text-text-default data-[state=active]:text-text-default disabled:pointer-events-none disabled:opacity-50 after:content-[''] after:absolute after:inset-x-0 after:-bottom-px after:h-0.5 after:bg-transparent data-[state=active]:after:bg-accent-bar",
      className
    )}
    {...props}
  />
));

TabsTrigger.displayName = TabsPrimitive.Trigger.displayName;

const TabsContent = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Content>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Content
    ref={ref}
    // Fades in over `--dur-fast-max`, `--ease-out` (`.br-tabs-content` in main.css).
    className={cn('br-tabs-content mt-2 animate-in fade-in', className)}
    {...props}
  />
));

TabsContent.displayName = TabsPrimitive.Content.displayName;

export { Tabs, TabsContent, TabsList, TabsTrigger };
