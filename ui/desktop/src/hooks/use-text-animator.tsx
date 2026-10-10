import SplitType from 'split-type';
import { useLayoutEffect, useRef } from 'react';
import { DUR, EASE_OUT, TRAVEL, prefersReducedMotion } from '../styles/motion';

/**
 * The unroll's timing, on the motion tokens (spec 2.4): each character slides
 * into place from the left over `--dur-med` on the one curve, 12ms after the one before it
 * (the same row step as the heatmap's reveal). Only `transform` and `opacity`
 * move (principle 9), so the old blur is gone.
 */
const CHAR_DURATION_MS = DUR.med;
const CHAR_STAGGER_MS = 12;
const CHAR_TRAVEL_PX = TRAVEL.small;

interface TextSplitterOptions {
  resizeCallback?: () => void;
  splitTypeTypes?: ('lines' | 'words' | 'chars')[];
}

// Class to split text into lines, words, and characters for animation
export class TextSplitter {
  textElement: HTMLElement;
  onResize: (() => void) | null;
  splitText: SplitType;
  previousContainerWidth: number | null = null;

  constructor(textElement: HTMLElement, options: TextSplitterOptions = {}) {
    if (!textElement || !(textElement instanceof HTMLElement)) {
      throw new Error('Invalid text element provided.');
    }

    const { resizeCallback, splitTypeTypes } = options;
    this.textElement = textElement;
    this.onResize = typeof resizeCallback === 'function' ? resizeCallback : null;

    const splitOptions = splitTypeTypes ? { types: splitTypeTypes } : {};
    this.splitText = new SplitType(this.textElement, splitOptions);

    if (this.onResize) {
      this.initResizeObserver();
    }
  }

  initResizeObserver() {
    // Use a simpler approach to avoid type issues
    const resizeObserver = new ResizeObserver(() => {
      // Just check the current width directly from the element
      if (this.textElement) {
        const currentWidth = Math.floor(this.textElement.getBoundingClientRect().width);

        if (this.previousContainerWidth && this.previousContainerWidth !== currentWidth) {
          this.splitText.split({ types: ['chars'] });
          this.onResize?.();
        }

        this.previousContainerWidth = currentWidth;
      }
    });

    resizeObserver.observe(this.textElement);
  }

  getLines(): HTMLElement[] {
    return this.splitText.lines ?? [];
  }

  getChars(): HTMLElement[] {
    return this.splitText.chars ?? [];
  }
}

export class TextAnimator {
  textElement: HTMLElement;
  splitter!: TextSplitter;
  originalChars!: string[];
  activeAnimations: globalThis.Animation[] = [];
  activeTimeouts: ReturnType<typeof setTimeout>[] = [];

  constructor(textElement: HTMLElement) {
    if (!textElement || !(textElement instanceof HTMLElement)) {
      throw new Error('Invalid text element provided.');
    }

    this.textElement = textElement;
    this.splitText();
  }

  private splitText() {
    this.splitter = new TextSplitter(this.textElement, {
      splitTypeTypes: ['words', 'chars'],
    });
    this.originalChars = this.splitter.getChars().map((char) => char.textContent || '');
  }

  animate() {
    this.reset();

    const chars = this.splitter.getChars();

    chars.forEach((char, position) => {
      char.style.opacity = '0';
      char.style.display = 'inline-block';
      char.style.position = 'relative';
      char.style.transform = `translateX(-${CHAR_TRAVEL_PX}px)`;

      const animation = char.animate(
        [
          { opacity: 0, transform: `translateX(-${CHAR_TRAVEL_PX}px)` },
          { opacity: 1, transform: 'none' },
        ],
        {
          duration: CHAR_DURATION_MS,
          easing: EASE_OUT,
          delay: position * CHAR_STAGGER_MS,
          iterations: 1,
          fill: 'forwards',
        }
      );

      this.activeAnimations.push(animation);

      animation.onfinish = () => {
        char.style.opacity = '1';
        char.style.transform = '';
      };
    });
  }

  reset() {
    // Clear all timeouts
    this.activeTimeouts.forEach((timeoutId) => clearTimeout(timeoutId));
    this.activeTimeouts = [];

    // Cancel all animations
    this.activeAnimations.forEach((animation) => animation.cancel());
    this.activeAnimations = [];

    // Reset text content
    const chars = this.splitter.getChars();
    chars.forEach((char, index) => {
      if (this.originalChars[index]) {
        char.textContent = this.originalChars[index];
      }
      char.style.opacity = '';
      char.style.transform = '';
    });
  }
}

interface UseTextAnimatorProps {
  /** When false the text renders immediately, unanimated. Defaults to true. */
  enabled?: boolean;
  text: string;
}

export function useTextAnimator({ text, enabled = true }: UseTextAnimatorProps) {
  const elementRef = useRef<HTMLSpanElement>(null);
  const animator = useRef<TextAnimator | null>(null);

  // ⚠ `useLayoutEffect`, and NOT `useEffect`, because the first frame the user
  // sees is the whole point of an unroll.
  //
  // React paints the heading with its text at full opacity; `animate()` is what
  // hides the characters so they can arrive. Run that from `useEffect` and the
  // browser has already painted the finished sentence before the hook gets a
  // turn — so the arrival reads as: the answer, a blink of nothing, then the
  // answer again, slowly. The reported symptom on opening a new window, and it
  // was two stacked delays rather than one: a post-paint effect, and then a
  // further 100 ms `setTimeout` "to ensure content is ready" with the finished
  // text on screen the whole time. A layout effect runs after the DOM is
  // committed and BEFORE the paint, so the first frame already has the
  // characters hidden with their animations running.
  //
  // The 100 ms bought nothing to begin with: `TextAnimator`'s constructor
  // splits synchronously, and this split is `['words', 'chars']` with no
  // `lines`, so nothing here depends on a measured layout (or on a webfont
  // having settled) the way a line split would.
  //
  // ⚠ Hiding belongs HERE and not in the rendered markup. A `style={{opacity:
  // 0}}` on the span would also kill the flash, and would leave the greeting
  // permanently invisible for anyone whose branch below returns early —
  // `enabled: false`, or `prefers-reduced-motion` — and for anyone whose
  // animator throws. Nothing that has to run in order for text to be *visible*
  // should live in an effect.
  useLayoutEffect(() => {
    if (!elementRef.current) return;

    // ⚠ The caller decides WHETHER, this hook decides HOW. The greeting plays
    // its unroll on a new chat and stays still on every remount of the same
    // one, and that gate lives at the call site because only it knows which
    // chat this is.
    if (!enabled) return;

    if (prefersReducedMotion()) {
      return;
    }

    animator.current = new TextAnimator(elementRef.current);
    animator.current.animate();

    // Cleanup
    return () => {
      if (animator.current) {
        animator.current.reset();
      }
    };
  }, [text, enabled]); // Re-run when text changes

  return elementRef;
}
