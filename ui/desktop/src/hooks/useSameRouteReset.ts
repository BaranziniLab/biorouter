import { useEffect, useRef } from 'react';

/**
 * "Take me back to the top of the page I am already on."
 *
 * Clicking a sidebar item you are already on was a dead click. React Router
 * reconciles a same-path navigation rather than remounting the route element,
 * so a page holding sub-state — a schedule's run detail, and the session
 * history nested inside it — stayed exactly where it was, and the only escape
 * was the in-page Back. The row is lit the whole time, which is the app telling
 * the user "this is the destination"; pressing it should get them the
 * destination.
 *
 * ⚠ **This is a window event, not route state, and that is deliberate.**
 * Navigating to the same path with a fresh `state` object would work, but the
 * cost is paid somewhere unrelated: several routes read `location.state` as
 * their view options with a `|| window.history.state` fallback
 * (`App.tsx`'s Settings and Extensions routes both do), and a truthy state
 * object shadows that fallback. A reset is also not a navigation — it adds no
 * destination worth a history entry — so an event models it honestly and
 * touches no router behaviour at all.
 */
export const SAME_ROUTE_RESET_EVENT = 'biorouter:same-route-reset';

export interface SameRouteResetDetail {
  /** The route path the user re-selected. */
  path: string;
}

/** Announce that the user re-selected the route they are already on. */
export function announceSameRouteReset(path: string): void {
  window.dispatchEvent(
    new CustomEvent<SameRouteResetDetail>(SAME_ROUTE_RESET_EVENT, { detail: { path } })
  );
}

/**
 * Run `onReset` when the user re-selects `path` from the rail.
 *
 * For a page that drills into a sub-view it owns in local state. A page whose
 * every view is a URL has nothing to do here and should not call this.
 */
export function useSameRouteReset(path: string, onReset: () => void): void {
  // The callback is read from a ref so a caller passing an inline arrow — which
  // is every caller — does not tear the listener down and rebuild it on each
  // render.
  const handler = useRef(onReset);
  handler.current = onReset;

  useEffect(() => {
    const listener = (event: Event) => {
      if ((event as CustomEvent<SameRouteResetDetail>).detail?.path !== path) return;
      handler.current();
    };
    window.addEventListener(SAME_ROUTE_RESET_EVENT, listener);
    return () => window.removeEventListener(SAME_ROUTE_RESET_EVENT, listener);
  }, [path]);
}
