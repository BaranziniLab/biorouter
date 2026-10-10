import { useLayoutEffect, useRef } from 'react';
import { DUR, EASE_OUT, prefersReducedMotion } from '../styles/motion';

/**
 * The greeting's arrival: ONE fade of the whole line, `--dur-fast-max` on the
 * one curve, opacity only (spec 2.4 and principle 9).
 *
 * It used to be a per-character unroll: `split-type` cut the sentence into one
 * element per letter and each slid in 12ms after the one before, so a 44-letter
 * question took about 950ms to finish and read "Which patient trends are
 * waiting to" a third of a second in. That is 44 staggered items against the
 * spec's cap of five, and the vision review measured it replaying on every
 * return to Home. One element fading is inside every rule, needs no split, and
 * leaves the heading's text a single node a screen reader reads as a sentence.
 */
const FADE_MS = DUR.fastMax;

interface UseTextAnimatorProps {
  /** When false the text renders immediately, unanimated. Defaults to true. */
  enabled?: boolean;
  text: string;
}

export function useTextAnimator({ text, enabled = true }: UseTextAnimatorProps) {
  const elementRef = useRef<HTMLSpanElement>(null);

  // ⚠ `useLayoutEffect`, and NOT `useEffect`, because the first frame the user
  // sees is the whole point of an arrival.
  //
  // React paints the heading with its text at full opacity; the animation is
  // what holds it at 0 until it fades in. Started from `useEffect`, the browser
  // has already painted the finished sentence before the hook gets a turn, so
  // the arrival reads as: the answer, a blink of nothing, then the answer
  // again. A layout effect runs after the DOM is committed and BEFORE the
  // paint, so the first frame already holds the animation's first keyframe.
  //
  // ⚠ Hiding belongs HERE and not in the rendered markup. A `style={{opacity:
  // 0}}` on the span would also kill the flash, and would leave the greeting
  // permanently invisible for anyone whose branch below returns early
  // (`enabled: false`, `prefers-reduced-motion`, no Web Animations). The
  // animation's `backwards` fill only holds opacity while it runs; when it
  // ends or is cancelled the text is simply visible, with no inline style left
  // behind to undo.
  useLayoutEffect(() => {
    const element = elementRef.current;
    if (!element) return;

    // ⚠ The caller decides WHETHER, this hook decides HOW. The greeting fades
    // in on an arrival and stays still on every remount of the same one, and
    // that gate lives at the call site because only it knows which arrival this
    // is.
    if (!enabled) return;
    if (prefersReducedMotion()) return;
    if (typeof element.animate !== 'function') return;

    const fade = element.animate([{ opacity: 0 }, { opacity: 1 }], {
      duration: FADE_MS,
      easing: EASE_OUT,
      fill: 'backwards',
    });
    return () => fade.cancel();
  }, [text, enabled]);

  return elementRef;
}
