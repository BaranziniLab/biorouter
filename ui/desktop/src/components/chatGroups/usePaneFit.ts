import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import { flushSync } from 'react-dom';
import { PANE_FULL_UNMEASURED, paneFit, type PaneFitAnswer } from '../Layout/yieldLadder';

export interface PaneFitState {
  answer: PaneFitAnswer;
  /** True only when a measured full pane has just turned compact in this mount. */
  animateIn: boolean;
}

/**
 * Rung 5's observer: is this pane full or compact?
 *
 * Every sample reads the PANE'S OWN BOX (`getBoundingClientRect`, never the
 * window) and ignores the observer's entries, so it is right under a fake
 * observer and in jsdom, where every rect is 0 and every pane reads as
 * unmeasured, which is full.
 *
 * It re-renders only when the fit or the shortfall changes: a splitter drag
 * samples every frame and `paneFit` returns the same answer almost every time,
 * so the chat behind it renders once per crossing, never once per frame.
 *
 * NO FRAME OF A BROKEN CHAT. A crossing seen by the observer is committed with
 * `flushSync`, so the placeholder and the hidden content land before that frame
 * paints. The first sample runs in a layout effect, which React flushes before
 * paint, so a reload into a narrow layout shows the placeholder from the first
 * frame. The same effect samples again when `alone` changes, because a pane
 * that becomes the only one is full at once.
 */
export function usePaneFit(
  paneRef: React.RefObject<HTMLElement | null>,
  opts: {
    alone: boolean;
    /** Called synchronously before the state changes, only when fit or shortfall changes. */
    onWillChange?: (from: PaneFitAnswer, to: PaneFitAnswer) => void;
  }
): PaneFitState {
  const [state, setState] = useState<PaneFitState>({
    answer: PANE_FULL_UNMEASURED,
    animateIn: false,
  });
  const answerRef = useRef<PaneFitAnswer | null>(null);
  const aloneRef = useRef(opts.alone);
  aloneRef.current = opts.alone;
  const onWillChangeRef = useRef(opts.onWillChange);
  onWillChangeRef.current = opts.onWillChange;

  const sample = useCallback(
    (commit: 'sync' | 'layout') => {
      const pane = paneRef.current;
      if (!pane) return;
      const box = pane.getBoundingClientRect();
      const previous = answerRef.current;
      const from = previous ?? PANE_FULL_UNMEASURED;
      const next = paneFit({
        width: box.width,
        height: box.height,
        alone: aloneRef.current,
        previous,
      });
      answerRef.current = next;
      if (from.fit === next.fit && from.shortfall === next.shortfall) return;
      onWillChangeRef.current?.(from, next);
      const update = {
        answer: next,
        // "Mounting is not opening": a pane that mounts compact appears at rest.
        animateIn: from.fit === 'full' && next.fit === 'compact' && from.measured,
      };
      if (commit === 'sync') flushSync(() => setState(update));
      else setState(update);
    },
    [paneRef]
  );

  useLayoutEffect(() => {
    sample('layout');
  }, [opts.alone, sample]);

  useLayoutEffect(() => {
    if (typeof ResizeObserver === 'undefined') {
      const onResize = () => sample('sync');
      window.addEventListener('resize', onResize);
      return () => window.removeEventListener('resize', onResize);
    }
    const pane = paneRef.current;
    if (!pane) return;
    const observer = new ResizeObserver(() => sample('sync'));
    observer.observe(pane);
    return () => observer.disconnect();
  }, [paneRef, sample]);

  return state;
}
