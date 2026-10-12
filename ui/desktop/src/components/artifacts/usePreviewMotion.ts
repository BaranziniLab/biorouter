import { useEffect, useLayoutEffect, useRef, type RefObject } from 'react';
import type { PreviewPanelMode } from '../Layout/yieldLadder';
import { DUR, EASE_OUT, TRAVEL, isWindowResizing, prefersReducedMotion } from '../../styles/motion';

/** The body's three motions, in ms: the panel tier in, the fast tier out (spec 2.4). */
export const PREVIEW_MOTION = { open: DUR.med, close: DUR.fast, orientation: DUR.medMin } as const;

/** One WAAPI keyframe (`Keyframe`, spelled out: the DOM lib's name is not an ESLint global). */
type PreviewKeyframe = { opacity: number; transform?: string };

/**
 * The keyframes for one change of the preview body (spec 3.8, motion F1 to F4).
 *
 * - Open beside the chat: in from 32px right while fading in. Open as a sheet:
 *   down 16px from under the header (the sheet sits close to it).
 * - Close: half the travel, faster. A sheet only fades: it never leaves the way
 *   it came, because there is no room above it to go into.
 * - Side ↔ stack when an ACTION crossed the seam (a split opened, the sidebar
 *   toggled): a crossfade with no travel, since the body did not come from
 *   anywhere new. A resize crossing snaps; see `usePreviewMotion`.
 */
export function previewMotionKeyframes(
  change: 'open' | 'close' | 'orientation',
  layout: PreviewPanelMode
): PreviewKeyframe[] {
  if (change === 'orientation') return [{ opacity: 0 }, { opacity: 1 }];
  if (change === 'open') {
    const offset =
      layout === 'stack'
        ? `translateY(-${TRAVEL.previewOut}px)`
        : `translateX(${TRAVEL.previewIn}px)`;
    return [
      { opacity: 0, transform: offset },
      { opacity: 1, transform: 'none' },
    ];
  }
  if (layout === 'stack') return [{ opacity: 1 }, { opacity: 0 }];
  return [
    { opacity: 1, transform: 'none' },
    { opacity: 0, transform: `translateX(${TRAVEL.previewOut}px)` },
  ];
}

/** The panel the body sits in, which wears the `data-preview-motion` marker. */
export const PREVIEW_PANEL_SELECTOR = '.br-preview-panel';

/** Animate the existing content layer; changing grid geometry never moves focus or reloads a frame. */
export function usePreviewMotion(
  ref: RefObject<HTMLDivElement | null>,
  { isOpen, layout, ready }: { isOpen: boolean; layout: PreviewPanelMode; ready: boolean }
) {
  const previous = useRef<{ open: boolean; layout: PreviewPanelMode }>({ open: false, layout });
  const active = useRef<{ animation: Animation; opening: boolean } | null>(null);

  useEffect(() => {
    const preference = window.matchMedia('(prefers-reduced-motion: reduce)');
    const cancel = () => {
      active.current?.animation.cancel();
      active.current = null;
      const panel = ref.current?.closest<HTMLElement>(PREVIEW_PANEL_SELECTOR);
      if (panel) delete panel.dataset.previewMotion;
    };
    const changed = () => {
      if (preference.matches) cancel();
    };
    preference.addEventListener('change', changed);
    return () => {
      preference.removeEventListener('change', changed);
      cancel();
    };
  }, [ref]);

  useLayoutEffect(() => {
    if (isOpen && !ready) return;
    const before = previous.current;
    previous.current = { open: isOpen, layout };
    const body = ref.current;
    if (!body || !body.animate || prefersReducedMotion()) return;
    if (before.open === isOpen && (!isOpen || before.layout === layout)) return;
    // An OS auto-resize can cross the seam during entrance. Keep that entrance running.
    if (isOpen && active.current?.opening && active.current.animation.playState === 'running')
      return;
    const opening = isOpen && !before.open;
    const change: 'open' | 'close' | 'orientation' = opening
      ? 'open'
      : isOpen
        ? 'orientation'
        : 'close';
    // Layout snaps against a resize (principle 9): a crossing the window or the
    // sidebar drag caused is a new shape, not an event, so nothing plays.
    if (change === 'orientation' && isWindowResizing()) return;
    active.current?.animation.cancel();
    const animation = body.animate(previewMotionKeyframes(change, layout), {
      duration: PREVIEW_MOTION[change],
      easing: EASE_OUT,
      fill: 'both',
    });
    active.current = { animation, opening };
    // NOTHING POPS (spec 3.8). The panel around the body (its strip, its bar,
    // its seam) fades on the same beat, in authored CSS keyed on this marker
    // (`preview-panel.css`). A CSS transition cannot do it on open: the panel
    // is inserted and opened before its first style pass, so there is no
    // "before" to transition from, and the strip appeared in one frame.
    const panel = body.closest<HTMLElement>(PREVIEW_PANEL_SELECTOR);
    if (panel) panel.dataset.previewMotion = change;
    const settle = () => {
      if (panel && active.current?.animation === animation) delete panel.dataset.previewMotion;
    };
    animation.onfinish = () => {
      if (active.current?.animation !== animation) return;
      settle();
      animation.cancel();
      active.current = null;
    };
  }, [isOpen, layout, ready, ref]);
}
