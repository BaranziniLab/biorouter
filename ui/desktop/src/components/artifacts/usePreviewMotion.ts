import { useEffect, useLayoutEffect, useRef, type RefObject } from 'react';
import type { PreviewPanelMode } from '../Layout/yieldLadder';

export const PREVIEW_MOTION = { open: 300, close: 125, orientation: 250 } as const;

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
    };
    const changed = () => {
      if (preference.matches) cancel();
    };
    preference.addEventListener('change', changed);
    return () => {
      preference.removeEventListener('change', changed);
      cancel();
    };
  }, []);

  useLayoutEffect(() => {
    if (isOpen && !ready) return;
    const before = previous.current;
    previous.current = { open: isOpen, layout };
    const body = ref.current;
    if (!body || !body.animate || window.matchMedia('(prefers-reduced-motion: reduce)').matches)
      return;
    if (before.open === isOpen && (!isOpen || before.layout === layout)) return;
    // An OS auto-resize can cross the seam during entrance. Keep that entrance running.
    if (isOpen && active.current?.opening && active.current.animation.playState === 'running')
      return;
    active.current?.animation.cancel();
    const offset = layout === 'stack' ? 'translateY(-32px)' : 'translateX(32px)';
    const opening = isOpen && !before.open;
    const animation = body.animate(
      isOpen
        ? [
            { opacity: 0, transform: offset },
            { opacity: 1, transform: 'none' },
          ]
        : [
            { opacity: 1, transform: 'none' },
            { opacity: 0, transform: offset },
          ],
      {
        duration: isOpen
          ? opening
            ? PREVIEW_MOTION.open
            : PREVIEW_MOTION.orientation
          : PREVIEW_MOTION.close,
        easing: 'cubic-bezier(0.2, 0, 0, 1)',
        fill: 'both',
      }
    );
    active.current = { animation, opening };
    animation.onfinish = () => {
      if (active.current?.animation !== animation) return;
      animation.cancel();
      active.current = null;
    };
  }, [isOpen, layout, ready, ref]);
}
