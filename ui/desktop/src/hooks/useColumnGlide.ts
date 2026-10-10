import { useCallback, useEffect, useMemo, useRef, type RefObject } from 'react';
import { EASE_OUT, MOTION, isWindowResizing, prefersReducedMotion } from '../styles/motion';

/**
 * THE CONVERSATION GLIDE.
 *
 * When the summary rail or a side preview takes its column, the grid changes in
 * ONE frame (the layout snaps) and the conversation then glides from where it
 * was to where it is now: a FLIP with `transform` only. A width tween would
 * re-lay out a transcript that is not virtualised on every frame, which is
 * exactly the horizontal jitter Codex shipped with its pinned summary.
 *
 * What moves are the GRID ITEMS, the transcript cell and the composer bar, not
 * `.biorouter-chat-column` inside the scroller: a transform on the column would
 * add to the viewport's scrollable overflow and flash a horizontal scrollbar.
 * The split box clips (`overflow: clip`), so a cell's overshoot is bounded.
 *
 * It never animates against geometry: nothing plays while the window or the
 * sidebar is being resized, under reduced motion, for a move under 8px, while
 * the window grows to seat a preview, or when the split box's own width changed
 * between the capture and the commit (the change was the window's, not the
 * person's).
 */

const TRANSCRIPT = "[data-preview-area='transcript']";
const COMPOSER = "[data-preview-area='composer']";
const TRANSCRIPT_MEASURE = '.biorouter-chat-column, .biorouter-clean-conversation-content';
const COMPOSER_MEASURE = '[data-composer-shell]';

/** A capture older than this is not the "before" of anything that commits now. */
export const GLIDE_CAPTURE_TTL_MS = 1000;

type Capture = {
  at: number;
  splitWidth: number;
  transcriptLeft: number | null;
  composerLeft: number | null;
  windowGrowing: boolean;
};

export interface GlideOptions {
  /** The glide's length, in ms (`DUR.med` to open, `DUR.fastMax` to close). */
  duration: number;
  /** Also move the composer bar. False when the composer is already moving. */
  composer?: boolean;
}

export interface ColumnGlide {
  /**
   * Read where the conversation is, BEFORE an action changes the grid (a
   * preview opening or closing). The matching `playCaptured` after the commit
   * moves it from there.
   */
  capture: (options?: { windowGrowing?: boolean }) => void;
  /**
   * Play the pending capture against the layout that has just committed. Call
   * from a layout effect. Returns whether a capture was consumed.
   */
  playCaptured: (options: GlideOptions) => boolean;
  /** Glide by a known distance: the summary rail, whose `dx` is exact. */
  glideBy: (dx: number, options: GlideOptions) => void;
}

function leftOf(element: Element | null | undefined): number | null {
  if (!element) return null;
  const rect = element.getBoundingClientRect();
  return Number.isFinite(rect.left) ? rect.left : null;
}

/** The translateX an in-flight glide holds an element at, so a new one starts there. */
function currentTranslateX(element: HTMLElement): number {
  try {
    const transform = getComputedStyle(element).transform;
    if (!transform || transform === 'none') return 0;
    const match = transform.match(/matrix(?:3d)?\(([^)]+)\)/);
    if (!match) return 0;
    const values = match[1].split(',').map((value) => Number.parseFloat(value));
    const x = values.length === 16 ? values[12] : values[4];
    return Number.isFinite(x) ? x : 0;
  } catch {
    return 0;
  }
}

function isAnimating(element: Element | null): boolean {
  if (!element || typeof (element as HTMLElement).getAnimations !== 'function') return false;
  return (element as HTMLElement)
    .getAnimations()
    .some((animation) => animation.playState === 'running' || animation.pending);
}

export function useColumnGlide(splitPaneRef: RefObject<HTMLElement | null>): ColumnGlide {
  const pendingRef = useRef<Capture | null>(null);
  const runningRef = useRef(new Map<Element, Animation>());

  const stopAll = useCallback(() => {
    for (const animation of runningRef.current.values()) animation.cancel();
    runningRef.current.clear();
  }, []);

  useEffect(() => stopAll, [stopAll]);

  const animate = useCallback((element: HTMLElement, dx: number, duration: number) => {
    const running = runningRef.current;
    // Start a new glide from where an unfinished one holds the element, so a
    // quick second toggle continues the motion rather than jumping.
    const offset = running.has(element) ? currentTranslateX(element) : 0;
    running.get(element)?.cancel();
    running.delete(element);
    const from = dx + offset;
    if (Math.abs(from) < MOTION.minGlidePx || typeof element.animate !== 'function') return;
    const animation = element.animate(
      [{ transform: `translateX(${from}px)` }, { transform: 'translateX(0)' }],
      { duration, easing: EASE_OUT }
    );
    running.set(element, animation);
    const forget = () => {
      if (running.get(element) === animation) running.delete(element);
    };
    animation.onfinish = forget;
    animation.oncancel = forget;
  }, []);

  const blocked = useCallback(() => isWindowResizing() || prefersReducedMotion(), []);

  const capture = useCallback(
    (options: { windowGrowing?: boolean } = {}) => {
      const split = splitPaneRef.current;
      if (!split) {
        pendingRef.current = null;
        return;
      }
      const transcript = split.querySelector(TRANSCRIPT);
      const composer = split.querySelector(COMPOSER);
      pendingRef.current = {
        at: performance.now(),
        splitWidth: split.clientWidth,
        transcriptLeft: leftOf(transcript?.querySelector(TRANSCRIPT_MEASURE) ?? transcript),
        composerLeft: leftOf(composer?.querySelector(COMPOSER_MEASURE) ?? composer),
        windowGrowing: options.windowGrowing === true,
      };
    },
    [splitPaneRef]
  );

  const playCaptured = useCallback(
    ({ duration, composer: moveComposer = true }: GlideOptions) => {
      const pending = pendingRef.current;
      pendingRef.current = null;
      const split = splitPaneRef.current;
      if (!pending || !split) return false;
      if (performance.now() - pending.at > GLIDE_CAPTURE_TTL_MS) return false;
      if (pending.windowGrowing || pending.splitWidth !== split.clientWidth || blocked()) {
        return true;
      }
      const transcript = split.querySelector<HTMLElement>(TRANSCRIPT);
      if (transcript && pending.transcriptLeft !== null) {
        // Cancel first: the "after" rect must not include an old glide's offset.
        runningRef.current.get(transcript)?.cancel();
        const after = leftOf(transcript.querySelector(TRANSCRIPT_MEASURE) ?? transcript);
        if (after !== null) animate(transcript, pending.transcriptLeft - after, duration);
      }
      const bar = split.querySelector<HTMLElement>(COMPOSER);
      const shell = bar?.querySelector(COMPOSER_MEASURE) ?? null;
      if (bar && moveComposer && pending.composerLeft !== null && !isAnimating(shell)) {
        runningRef.current.get(bar)?.cancel();
        const after = leftOf(shell ?? bar);
        if (after !== null) animate(bar, pending.composerLeft - after, duration);
      }
      return true;
    },
    [animate, blocked, splitPaneRef]
  );

  const glideBy = useCallback(
    (dx: number, { duration, composer: moveComposer = true }: GlideOptions) => {
      const split = splitPaneRef.current;
      if (!split || blocked() || Math.abs(dx) < MOTION.minGlidePx) return;
      const transcript = split.querySelector<HTMLElement>(TRANSCRIPT);
      if (transcript) animate(transcript, dx, duration);
      const bar = split.querySelector<HTMLElement>(COMPOSER);
      // The composer's own FLIP (the first turn's move from the centred empty
      // state) already lands on the final rect, rail included; a second
      // transform on top of it would double the move.
      if (bar && moveComposer && !isAnimating(bar.querySelector(COMPOSER_MEASURE))) {
        animate(bar, dx, duration);
      }
    },
    [animate, blocked, splitPaneRef]
  );

  return useMemo(() => ({ capture, playCaptured, glideBy }), [capture, playCaptured, glideBy]);
}

/**
 * Keep the artifact card the person clicked where it was while the transcript
 * narrows for a side preview: read its offset from the viewport's top before the
 * commit, and scroll by the difference after it. A transcript that follows its
 * bottom keeps doing that instead.
 */
export function captureScrollAnchor(
  viewport: HTMLElement | null | undefined,
  candidate: Element | null
): (() => void) | null {
  if (!viewport || !candidate || !viewport.contains(candidate) || candidate === viewport) {
    return null;
  }
  const offset = () => candidate.getBoundingClientRect().top - viewport.getBoundingClientRect().top;
  const before = offset();
  return () => {
    if (!candidate.isConnected) return;
    const delta = offset() - before;
    if (Math.abs(delta) >= 1) viewport.scrollTop += delta;
  };
}
