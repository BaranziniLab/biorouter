import { useCallback, useEffect, useRef, useState } from 'react';

/**
 * A flag that raises itself, then lowers itself again `durationMs` later —
 * "Copied!", "Saved", a control disabled while the OS catches up.
 *
 * The app had ~12 hand-rolled copies of this, every one of them spelled
 * `setCopied(true); setTimeout(() => setCopied(false), 2000)` inside a click
 * handler, and every one of them wrong in the same two ways:
 *
 *  * **Nothing cancelled the timer on unmount.** A modal closed, a toast
 *    dismissed or a route changed inside the 2s window left a callback holding a
 *    setter for a tree that no longer exists. React logs it; under a test runner
 *    that has already torn jsdom down it is the `window is not defined` crash
 *    `SessionListView.revealTimer.test.tsx` was written for.
 *  * **Nothing cancelled the PREVIOUS timer on a re-trigger.** Copy twice inside
 *    two seconds and the first timer still fires on schedule, so "Copied!"
 *    disappears roughly a second after the second copy rather than two seconds
 *    after it. The visible flag is a lie about the most recent action.
 *
 * Both are closed here, once. A handler calls `raise()` and owns no timer.
 *
 * ⚠ The unmount cleanup reads a REF, and the effect has an empty dependency
 * list on purpose. Clearing the timer in a `useEffect` keyed on the flag would
 * cancel the countdown on the render the flag is raised in — the flag would
 * never come down on its own — which is the plausible wrong shape of this hook.
 */
export function useTransientValue<T>(
  durationMs: number
): readonly [T | null, (value: T) => void, () => void] {
  const [value, setValue] = useState<T | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clear = useCallback(() => {
    if (timerRef.current !== null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  // Unmount only. See the warning above.
  useEffect(() => clear, [clear]);

  const raise = useCallback(
    (next: T) => {
      clear();
      setValue(next);
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        setValue(null);
      }, durationMs);
    },
    [clear, durationMs]
  );

  const lower = useCallback(() => {
    clear();
    setValue(null);
  }, [clear]);

  return [value, raise, lower] as const;
}

/**
 * The boolean case, which is almost every call site: `const [copied, markCopied]
 * = useTransientFlag(2000)`.
 *
 * A thin wrapper over `useTransientValue` rather than a second implementation —
 * the whole point of this module is that there is one place where the timer is
 * cancelled, and two copies of that logic would be the drift it replaces.
 */
export function useTransientFlag(durationMs: number): readonly [boolean, () => void, () => void] {
  const [value, raise, lower] = useTransientValue<true>(durationMs);
  const mark = useCallback(() => raise(true), [raise]);
  return [value === true, mark, lower] as const;
}
