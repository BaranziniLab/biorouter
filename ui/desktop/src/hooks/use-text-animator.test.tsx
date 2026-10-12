import { render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DUR, EASE_OUT } from '../styles/motion';
import { useTextAnimator } from './use-text-animator';

/**
 * The arrival's contract is about the FIRST FRAME and about its size.
 *
 * The greeting is rendered with its text at full opacity and the animation is
 * what holds it at 0 until it fades in. That makes *when* the hook runs the
 * whole behaviour: a turn taken after the browser has painted shows the
 * finished sentence, then blanks it, then fades it in. A layout effect runs
 * before the paint, so the animation is already running in the first frame.
 *
 * jsdom never paints, so none of this can be observed the way a user observes
 * it. What jsdom *can* answer exactly is the question underneath: had the work
 * happened by the time the commit finished, or only later? These assert against
 * un-advanced fake timers, so "later" fails whether it is one frame later or a
 * hundred milliseconds later.
 */

const started: Array<{
  el: Element;
  delay: number;
  duration?: number;
  easing?: string;
  fill?: string;
  keyframes: Record<string, unknown>[];
}> = [];
let cancelled = 0;

function Harness({ text, enabled = true }: { text: string; enabled?: boolean }) {
  const ref = useTextAnimator({ text, enabled });
  return (
    <h1>
      <span ref={ref}>{text}</span>
    </h1>
  );
}

beforeEach(() => {
  started.length = 0;
  cancelled = 0;
  vi.useFakeTimers();
  // jsdom implements no Web Animations API. The stub records what was started
  // and returns just enough of an `Animation` for the animator to drive.
  Element.prototype.animate = function (
    this: Element,
    keyframes: Record<string, unknown>[],
    options: { delay?: number; duration?: number; easing?: string; fill?: string }
  ) {
    started.push({
      el: this,
      delay: options?.delay ?? 0,
      duration: options?.duration,
      easing: options?.easing,
      fill: options?.fill,
      keyframes,
    });
    return {
      cancel: () => {
        cancelled += 1;
      },
      onfinish: null,
    } as unknown as globalThis.Animation;
  } as unknown as Element['animate'];
  window.matchMedia = ((q: string) => ({
    matches: false,
    media: q,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
});

afterEach(() => {
  vi.useRealTimers();
});

describe('useTextAnimator', () => {
  it('has started the fade before the first paint', () => {
    const { container } = render(<Harness text="Hello there" />);

    // NOTHING is advanced between the render and this assertion. Anything the
    // hook defers (a `setTimeout`, or simply being a `useEffect` that the
    // browser paints ahead of) leaves it unsatisfied.
    expect(started).toHaveLength(1);
    expect(started[0].el).toBe(container.querySelector('span'));
    // `backwards` holds the first keyframe only while the fade runs.
    expect(started[0].fill).toBe('backwards');
  });

  /**
   * One element, one fade (spec 2.4: a stagger animates at most five items).
   * The per-character unroll it replaced split a 44-letter question into 44
   * staggered elements that took about 950ms to finish.
   */
  it('fades the whole line once: opacity only, the fast-max tier on the one curve', () => {
    const { container } = render(
      <Harness text="Which patient trends are waiting to be discovered?" />
    );
    expect(started).toHaveLength(1);
    const [fade] = started;
    expect(fade.duration).toBe(DUR.fastMax);
    expect(fade.easing).toBe(EASE_OUT);
    expect(fade.delay).toBe(0);
    expect(fade.keyframes).toEqual([{ opacity: 0 }, { opacity: 1 }]);
    // No split: the text stays one node, so it is read as a sentence.
    expect(container.querySelector('span')?.childNodes).toHaveLength(1);
    expect(container.querySelector('span')?.getAttribute('style')).toBeNull();
  });

  it('cancels the fade when the heading unmounts, leaving nothing behind', () => {
    const { unmount } = render(<Harness text="Hello there" />);
    unmount();
    expect(cancelled).toBe(1);
  });

  /**
   * ⚠ The two ways out of this hook must leave the text VISIBLE, which is why
   * the hiding lives in the animator and not in the rendered markup. A
   * `style={{opacity: 0}}` on the span would kill the flash just as well and
   * would make these two cases render nothing at all, permanently.
   */
  it('leaves the text alone when animation is switched off at the call site', () => {
    const { container } = render(<Harness text="Hello there" enabled={false} />);
    expect(started).toHaveLength(0);
    expect(container.querySelector('span')?.style.opacity).not.toBe('0');
    expect(container.textContent).toBe('Hello there');
  });

  it('leaves the text alone under prefers-reduced-motion', () => {
    window.matchMedia = ((q: string) => ({
      matches: true,
      media: q,
      addEventListener: () => {},
      removeEventListener: () => {},
    })) as unknown as typeof window.matchMedia;

    const { container } = render(<Harness text="Hello there" />);
    expect(started).toHaveLength(0);
    expect(container.querySelector('span')?.style.opacity).not.toBe('0');
    expect(container.textContent).toBe('Hello there');
  });
});
