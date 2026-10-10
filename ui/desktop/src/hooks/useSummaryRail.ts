import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
} from 'react';
import {
  summaryRailFit,
  summaryRailVisible,
  summaryToggleMode,
  type PreviewPanelMode,
  type SummaryRailFit,
  type SummaryRailPreference,
} from '../components/Layout/yieldLadder';
import { useSummaryRailPreference } from '../components/Layout/summaryRailPreference';

/**
 * How long a width that would SHOW the rail must hold still before the rail
 * appears. A hide is immediate: the conversation's measure is protected first.
 * The delay keeps the column from jumping halfway through someone else's motion,
 * such as the sidebar's 300ms collapse sweeping the pane through the threshold.
 */
export const SUMMARY_RAIL_SHOW_SETTLE_MS = 160;

/**
 * The longest a closing rail keeps its column while its card plays the exit
 * (`--dur-fast`, 125ms) before it is removed anyway. The card normally reports
 * `animationend` first; this covers a card whose animation never ran (a resize
 * zeroes animations, a hidden window throttles them).
 */
export const SUMMARY_RAIL_EXIT_FALLBACK_MS = 200;

/** Spread onto the split box beside the artifact panel's own props. */
export interface SummaryRailSplitPaneProps {
  'data-summary-rail'?: '';
  style?: CSSProperties;
}

export interface SummaryRailController {
  /** Rung 0's answer for this pane right now. */
  fit: SummaryRailFit;
  /** The rail is wanted, active, fits and this is not a phone-width browser. */
  shown: boolean;
  /** Shown, or still playing its exit after the person closed it. */
  rendered: boolean;
  /** The card's state: `closed` only while it plays its exit. */
  state: 'open' | 'closed';
  /**
   * True when the rail appears because of geometry (a resize, the sidebar, a
   * preview, a tab switch, a reload): it then appears at rest. False only for
   * the commit in which the person opened it or the chat's first turn started.
   * The card reads it once, at mount.
   */
  still: boolean;
  /** What the header button does: toggle the rail, or open the popover. */
  mode: 'rail' | 'popover';
  preference: SummaryRailPreference;
  /** The width the grid gives the rail, in px. Meaningful while rendered. */
  width: number;
  /** Open or close the docked rail (rail mode only). */
  toggle: () => void;
  /** The card's exit has finished: remove the column. */
  onExited: () => void;
  splitPaneProps: SummaryRailSplitPaneProps;
}

export interface UseSummaryRailOptions {
  /** The split box: the box the conversation, the rail and the preview share. */
  splitPaneRef: RefObject<HTMLElement | null>;
  /** The chat has had a turn (or one is running). */
  active: boolean;
  isMobile: boolean;
  /** Null when no preview is mounted. */
  previewMode: PreviewPanelMode | null;
  /** The side preview's resolved width (`ArtifactPanelController.sideWidth`). */
  previewWidth: number;
  /**
   * Called after the commit in which the rail's column appeared or left because
   * of an action (the person's toggle, or the first turn). `dx` is how far the
   * centred conversation moved: half the rail's width, positive when the column
   * opened (the conversation was to the right of where it is now).
   *
   * Exact, not estimated: rung 0 only seats the rail when the conversation keeps
   * its full 760px measure on both sides of the change, and both the transcript
   * column and the composer are centred in the cell the rail narrows.
   */
  onGlide?: (dx: number, cause: 'open' | 'close') => void;
}

function sameFit(a: SummaryRailFit | null, b: SummaryRailFit): boolean {
  return !!a && a.fits === b.fits && a.width === b.width && a.measured === b.measured;
}

function readWidth(element: HTMLElement | null): number {
  const width = element?.clientWidth;
  return typeof width === 'number' && Number.isFinite(width) ? width : 0;
}

/**
 * Rung 0's state for one chat: whether the docked Chat summary rail is on
 * screen, how wide, and how it got there.
 *
 * The decision is derived during render from the measured pane width and the
 * preview's live mode and width, so a preview that opens hides the rail in the
 * SAME commit, and one that closes brings it back in the commit that removes the
 * panel. Only the pane width is observed (a `ResizeObserver` on the split box,
 * plus one synchronous read before the first paint so a tab switch never shows
 * the column centred and then jumping left). A width that would hide the rail is
 * applied at once; one that would show it waits for 160ms of stable width.
 *
 * It never resizes the window.
 */
export function useSummaryRail({
  splitPaneRef,
  active,
  isMobile,
  previewMode,
  previewWidth,
  onGlide,
}: UseSummaryRailOptions): SummaryRailController {
  const [preference, setPreference] = useSummaryRailPreference();
  const [paneWidth, setPaneWidth] = useState(0);
  // The previous committed answer, for the hysteresis. Null until this mount has
  // committed one: the first sample uses the base threshold.
  const [lastFit, setLastFit] = useState<SummaryRailFit | null>(null);
  const fit = summaryRailFit({ paneWidth, previewMode, previewWidth, previous: lastFit });
  const shown = summaryRailVisible({ fit, preference, active, isMobile });

  // "The first turn starts": `active` turning on after this chat mounted. A
  // chat that mounts already active (a tab switch, a reload) is at rest. This
  // is React's "store information from previous renders" pattern, so the
  // answer is known in the same render the rail first appears in.
  const [previousActive, setPreviousActive] = useState(active);
  const [animateEntrance, setAnimateEntrance] = useState(false);
  if (previousActive !== active) {
    setPreviousActive(active);
    if (active) setAnimateEntrance(true);
  }

  const [exiting, setExiting] = useState(false);
  // A rail that stops fitting while it exits leaves at once with the grid.
  const rendered = shown || (exiting && fit.fits);
  const renderedWidth = rendered ? fit.width : 0;
  const railMode = active && !isMobile ? summaryToggleMode(fit) : 'popover';

  // ---- Measurement --------------------------------------------------------

  // The latest inputs, for the observer's callback (it outlives renders).
  const inputsRef = useRef({ previewMode, previewWidth, lastFit, fit });
  inputsRef.current = { previewMode, previewWidth, lastFit, fit };
  const settleTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const observedRef = useRef<HTMLElement | null>(null);
  const observerRef = useRef<ResizeObserver | null>(null);

  const sample = useCallback((width: number) => {
    const {
      previewMode: mode,
      previewWidth: preview,
      lastFit: previous,
      fit: now,
    } = inputsRef.current;
    const next = summaryRailFit({
      paneWidth: width,
      previewMode: mode,
      previewWidth: preview,
      previous,
    });
    if (settleTimerRef.current) {
      clearTimeout(settleTimerRef.current);
      settleTimerRef.current = null;
    }
    if (!now.fits && next.fits) {
      // Show only once the width has held still.
      settleTimerRef.current = setTimeout(() => {
        settleTimerRef.current = null;
        setPaneWidth(width);
      }, SUMMARY_RAIL_SHOW_SETTLE_MS);
      return;
    }
    setPaneWidth(width);
  }, []);

  // Attach to whichever element is the split box now. Runs after every commit
  // but only does work when the element changed (an error screen replaces it).
  useLayoutEffect(() => {
    const element = splitPaneRef.current;
    if (element === observedRef.current) return;
    observerRef.current?.disconnect();
    observerRef.current = null;
    observedRef.current = element;
    if (!element) return;
    // Synchronous first read: React applies it before the first paint.
    setPaneWidth(readWidth(element));
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => sample(readWidth(element)));
    observer.observe(element);
    observerRef.current = observer;
  });

  useEffect(
    () => () => {
      observerRef.current?.disconnect();
      if (settleTimerRef.current) clearTimeout(settleTimerRef.current);
    },
    []
  );

  // Keep the hysteresis input current. Idempotent: re-deriving with the answer
  // as its own previous answer gives the same answer.
  useLayoutEffect(() => {
    if (!sameFit(lastFit, fit)) setLastFit(fit);
  }, [fit, lastFit]);

  // ---- Motion -------------------------------------------------------------

  const glideCauseRef = useRef<'close' | null>(null);
  const previousRenderedWidthRef = useRef(renderedWidth);
  const onGlideRef = useRef(onGlide);
  onGlideRef.current = onGlide;
  useLayoutEffect(() => {
    const previous = previousRenderedWidthRef.current;
    previousRenderedWidthRef.current = renderedWidth;
    const cause: 'open' | 'close' | null =
      glideCauseRef.current === 'close' && renderedWidth < previous
        ? 'close'
        : animateEntrance && renderedWidth > previous
          ? 'open'
          : null;
    glideCauseRef.current = null;
    if (!cause || previous === renderedWidth) return;
    onGlideRef.current?.((renderedWidth - previous) / 2, cause);
  }, [renderedWidth, animateEntrance]);

  // The entrance cause belongs to ONE commit: the card latched it at mount, and
  // a rail that appears later (a resize) must appear at rest.
  useLayoutEffect(() => {
    if (animateEntrance) setAnimateEntrance(false);
  }, [animateEntrance]);

  const exitingRef = useRef(exiting);
  exitingRef.current = exiting;
  const renderedWidthRef = useRef(renderedWidth);
  renderedWidthRef.current = renderedWidth;
  const finishExit = useCallback(() => {
    if (!exitingRef.current) return;
    // Only a column that is still on screen glides back; a rail the grid
    // already dropped (it stopped fitting mid-exit) has nothing left to move.
    if (renderedWidthRef.current > 0) glideCauseRef.current = 'close';
    setExiting(false);
  }, []);

  useEffect(() => {
    if (!exiting) return;
    const timer = setTimeout(finishExit, SUMMARY_RAIL_EXIT_FALLBACK_MS);
    return () => clearTimeout(timer);
  }, [exiting, finishExit]);

  const shownRef = useRef(shown);
  shownRef.current = shown;
  const toggle = useCallback(() => {
    if (shownRef.current) {
      setExiting(true);
      setPreference('closed');
    } else {
      setExiting(false);
      setAnimateEntrance(true);
      setPreference('open');
    }
  }, [setPreference]);

  const splitPaneProps: SummaryRailSplitPaneProps = rendered
    ? {
        'data-summary-rail': '',
        style: { '--summary-rail-width': `${renderedWidth}px` } as CSSProperties,
      }
    : {};

  return {
    fit,
    shown,
    rendered,
    state: shown ? 'open' : 'closed',
    still: !animateEntrance,
    mode: railMode,
    preference,
    width: renderedWidth,
    toggle,
    onExited: finishExit,
    splitPaneProps,
  };
}
