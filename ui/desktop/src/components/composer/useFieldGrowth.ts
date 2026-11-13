import { useEffect, type RefObject } from 'react';
import { DUR, EASE_OUT, prefersReducedMotion } from '../../styles/motion';

/**
 * The composer grows with what is typed, smoothly (owner, 2026-11-13: "when the
 * user is putting in text").
 *
 * The field keeps sizing itself natively (`field-sizing: content`), so nothing
 * is measured before a keystroke lands and the text never waits on a script.
 * What this adds is the one frame after: when the field's height changes by a
 * line, its box (`.br-composer-row__field`, which clips) animates from the old
 * height to the new one over `--dur-fast`, and the card, which is anchored at
 * its bottom edge, rises or settles with it instead of jumping a line at a
 * time. The ResizeObserver runs after layout and before paint, so the new
 * height is never painted ahead of its animation.
 *
 * ⚠ A named exception to "transform, opacity and clip-path only"
 * (`styles/motion.ts`): one small box's height, for 125ms, on a line change. A
 * transform cannot do it, because the card's height is what the transcript
 * above it gives way to.
 *
 * Skipped under reduced motion, and wherever `animate` is missing (jsdom).
 */
export function useFieldGrowth(fieldRef: RefObject<HTMLTextAreaElement | null>): void {
  useEffect(() => {
    const field = fieldRef.current;
    const box = field?.parentElement;
    if (!field || !box || typeof ResizeObserver === 'undefined') return;
    let settled = field.getBoundingClientRect().height;
    let running: Animation | null = null;

    const observer = new ResizeObserver(() => {
      const next = field.getBoundingClientRect().height;
      if (Math.abs(next - settled) < 1) return;
      // Mid-animation, start from where the box is drawn now, not where it was
      // headed, so a fast typist's second line continues the first's motion.
      const from = running ? box.getBoundingClientRect().height : settled;
      settled = next;
      running?.cancel();
      running = null;
      if (prefersReducedMotion() || typeof box.animate !== 'function') return;
      const animation = box.animate([{ height: `${from}px` }, { height: `${next}px` }], {
        duration: DUR.fast,
        easing: EASE_OUT,
      });
      running = animation;
      animation.onfinish = () => {
        if (running === animation) running = null;
      };
    });
    observer.observe(field);
    return () => {
      observer.disconnect();
      running?.cancel();
    };
  }, [fieldRef]);
}
