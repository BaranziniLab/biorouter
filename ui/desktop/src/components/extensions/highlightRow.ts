import { DUR } from '../../styles/motion';

/**
 * The one-shot wash on the row the person was just sent to (a deep link, a freshly installed
 * extension): add `.br-highlight` (WS-TOKENS, `main.css`), which takes `--overlay-selected` and
 * lets it go over three `--dur-slow` periods, and take the class off again when its animation
 * ends. It replaces an inline `box-shadow` ring painted with a colour alias that is being retired,
 * removed by a timer that had no story for reduced motion.
 *
 * Under reduced motion the stylesheet shows the wash for the same period and ends it with no
 * transition, so `animationend` fires then too. The timer is only the net for a host that never
 * runs the animation at all (a test, a hidden window), so the class cannot be left behind.
 *
 * Returns a cancel function that removes the class at once.
 */
export const HIGHLIGHT_CLASS = 'br-highlight';

/** Three `--dur-slow` periods plus a frame of slack: the longest the wash can run. */
export const HIGHLIGHT_FALLBACK_MS = 3 * DUR.slow + 100;

/** The wash running on each row, so sending the person to the same row twice restarts it. */
const running = new WeakMap<HTMLElement, () => void>();

export function highlightRow(element: HTMLElement): () => void {
  // Restart cleanly if the same row is sent to twice: end the first (its timer must not cut the
  // second short), remove, force a reflow, add.
  running.get(element)?.();
  element.classList.remove(HIGHLIGHT_CLASS);
  void element.offsetWidth;
  element.classList.add(HIGHLIGHT_CLASS);

  let timer: number | null = null;
  const done = () => {
    if (running.get(element) === done) running.delete(element);
    element.classList.remove(HIGHLIGHT_CLASS);
    element.removeEventListener('animationend', onEnd);
    if (timer !== null) window.clearTimeout(timer);
    timer = null;
  };
  // `animationend` bubbles, so a child's own animation (a spinner, an InfoTip) must not end it.
  const onEnd = (event: AnimationEvent) => {
    if (event.target === element) done();
  };
  element.addEventListener('animationend', onEnd);
  timer = window.setTimeout(done, HIGHLIGHT_FALLBACK_MS);
  running.set(element, done);
  return done;
}
