import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties, PointerEvent as ReactPointerEvent, RefObject } from 'react';
import { PREVIEW_MIN_WIDTH, previewPanelMode, type PreviewPanelMode } from '../Layout/yieldLadder';
import type { ArtifactSource } from './artifactTypes';

/**
 * The artifact side panel's geometry and open/close state, as one hook.
 *
 * WHY THIS IS A HOOK AND NOT INLINE IN A CHAT. The panel is the ONLY surface on
 * which a generated artifact is ever displayed — a `ui://` figure, an app
 * preview card, a file the agent wrote. Every surface that shows a transcript
 * therefore needs one: the live chat, the saved-session replay, and the shared
 * (read-only) session view. This state machine used to live inside `BaseChat`,
 * which is why the other two had no panel at all and fell back to rendering
 * figures inline in the message flow — a second renderer with its own CSP, its
 * own action channel and its own resize behaviour, silently diverging from this
 * one. One hook, three mounts, no second renderer.
 *
 * WHAT IS DELIBERATELY NOT HERE. Two behaviours are genuinely chat-only and stay
 * with the caller that has the state for them:
 *
 *   - **Auto-open.** Springing the panel on the newest artifact belongs to a
 *     LIVE turn. A saved transcript must open nothing until the reader clicks a
 *     card — `decideArtifactAutoOpen` in BaseChat encodes that rule and is not
 *     generalised here, because "never auto-open" needs no machinery.
 *   - **Auto-repair.** Feeding a render failure back to the agent needs a live
 *     conversation to feed it to. A read-only surface passes no `onRenderError`,
 *     and `ArtifactViewer` then never installs the listener at all.
 */

// Rung 2 of the yield ladder (D-32). The floor is shared with the ladder rather
// than re-declared, so the number the rule reasons about and the number the
// panel clamps to cannot drift apart.
const ARTIFACT_PANEL_MIN_WIDTH = PREVIEW_MIN_WIDTH;
const ARTIFACT_PANEL_MAX_WIDTH = 920;
// The primary column's PREFERRED width — what the panel gives up first. Below
// this the panel keeps narrowing toward its own 360px floor (the clamp in
// getMaxWidth), and at ARTIFACT_PANEL_MIN_WIDTH + CHAT_MIN_WIDTH there is
// nothing left to give and rung 2 fires. See previewPanelMode.
//
// One number for all three surfaces on purpose. A transcript's readable measure
// is wider than 640 anyway, so parameterising this would only create a way for
// the panel to behave differently depending on which surface you opened it
// from — the exact divergence this hook exists to end.
//
// ⚠ That "wider than 640" was written when a transcript's measure was 896px
// (the replay fork) or 1120px (the page measure). Both are gone: all three
// surfaces read `--measure-chat` = 760px now, so the margin is 120px rather
// than 250+. Still true, and still the right number — but it is the value to
// re-check first if the chat measure ever moves, because at 640 this floor
// stops being a floor under the column and becomes the column.
const ARTIFACT_PANEL_MIN_CHAT_WIDTH = 640;
const ARTIFACT_PANEL_DEFAULT_WIDTH_RATIO = 0.48;
const ARTIFACT_PANEL_AUTO_TUCK_WIDTH =
  ARTIFACT_PANEL_MIN_WIDTH + ARTIFACT_PANEL_MIN_CHAT_WIDTH + 48;
const ARTIFACT_PANEL_AUTO_EXPAND_PADDING = 24;
// Matches the panel's close transition (--motion-fast); exit is a tier faster
// than the --motion-base entrance so the panel unmounts as the slide completes.
const ARTIFACT_PANEL_EXIT_MS = 120;

function clampArtifactPanelWidth(value: number, max: number) {
  return Math.min(Math.max(value, ARTIFACT_PANEL_MIN_WIDTH), max);
}

export function getDefaultArtifactPanelWidth(containerWidth: number): number {
  const maxWidth = Math.max(
    ARTIFACT_PANEL_MIN_WIDTH,
    Math.min(ARTIFACT_PANEL_MAX_WIDTH, containerWidth - ARTIFACT_PANEL_MIN_CHAT_WIDTH)
  );
  return clampArtifactPanelWidth(
    Math.round(containerWidth * ARTIFACT_PANEL_DEFAULT_WIDTH_RATIO),
    maxWidth
  );
}

export function getArtifactPanelExpansionContentWidth(
  contentWidth: number,
  splitPaneWidth: number
): number | null {
  if (!Number.isFinite(contentWidth) || !Number.isFinite(splitPaneWidth)) return null;
  const deficit = ARTIFACT_PANEL_AUTO_TUCK_WIDTH - splitPaneWidth;
  if (deficit <= 0) return null;
  return Math.ceil(contentWidth + deficit + ARTIFACT_PANEL_AUTO_EXPAND_PADDING);
}

/**
 * The OS-window content width this surface needs to fit its artifact panel, or
 * null if it must not resize the window at all.
 *
 * Resizing the window is app-scoped, but the surfaces that host a panel are not.
 * When more than one is mounted (chat tabs and split panes do this), only the
 * focused one may resize — otherwise a background chat opening an artifact yanks
 * the window out from under the one the user is actually looking at. A saved
 * transcript never resizes: the reader opened a page, not a workspace.
 */
export function artifactPanelTargetContentWidth(opts: {
  isMobile: boolean;
  allowWindowResize: boolean;
  windowWidth: number;
  splitPaneWidth: number;
}): number | null {
  if (opts.isMobile) return null;
  if (!opts.allowWindowResize) return null;
  return getArtifactPanelExpansionContentWidth(opts.windowWidth, opts.splitPaneWidth) || null;
}

export interface UseArtifactPanelOptions {
  /**
   * Whether this surface is narrow enough that the panel always overlays rather
   * than taking a column. Callers pass `useIsMobile()`.
   */
  isMobile: boolean;
  /**
   * Grow the OS window to seat the panel when the pane is too narrow. Only the
   * focused chat may do this; read-only transcripts pass false.
   */
  allowWindowResize?: boolean;
}

/** Everything a host spreads onto `<ArtifactViewer>` so the three mounts cannot drift. */
export interface ArtifactViewerHostProps {
  artifact: ArtifactSource | null;
  isOpen: boolean;
  isResizing: boolean;
  onClose: () => void;
  onOpenArtifact: (artifact: ArtifactSource) => void;
  onResizeStart: ((event: ReactPointerEvent<HTMLDivElement>) => void) | undefined;
  style: CSSProperties | undefined;
  className: string;
}

export interface ArtifactPanelController {
  /**
   * Attach to the element that is the horizontal split container — the box the
   * primary column and the panel share. Rung 2 measures THIS, not the window: in
   * a split pane the two are decoupled, and a pane that cannot seat a 360px
   * transcript beside a 360px panel must overlay even in a roomy window.
   */
  splitPaneRef: RefObject<HTMLDivElement | null>;
  /** Null when no artifact is presented — the host skips the mount entirely. */
  artifact: ArtifactSource | null;
  previewMode: PreviewPanelMode;
  openArtifact: (artifact: ArtifactSource) => Promise<void>;
  closePanel: () => void;
  /** Tear the panel down without animating — for an identity change (new session). */
  reset: () => void;
  viewerProps: ArtifactViewerHostProps;
}

export function useArtifactPanel(options: UseArtifactPanelOptions): ArtifactPanelController {
  const { isMobile, allowWindowResize = false } = options;

  const [presentedArtifact, setPresentedArtifact] = useState<ArtifactSource | null>(null);
  const [isOpen, setIsOpen] = useState(false);
  const [isResizing, setIsResizing] = useState(false);
  const [width, setWidth] = useState<number | null>(null);
  // Rung 2 of the yield ladder. Derived from the PANE's measured width through
  // previewPanelMode — never stored as a raw width, so the state only changes on
  // the 720px crossing and a splitter drag does not re-render the whole surface
  // once per frame.
  const [previewMode, setPreviewMode] = useState<PreviewPanelMode>('side');

  const splitPaneRef = useRef<HTMLDivElement>(null);
  const closeTimerRef = useRef<number | null>(null);
  const openFrameRef = useRef<number | null>(null);
  const resizeFrameRef = useRef<number | null>(null);
  const pendingWidthRef = useRef<number | null>(null);
  const resizeCleanupRef = useRef<(() => void) | null>(null);
  const widthUserSetRef = useRef(false);

  useEffect(() => {
    return () => {
      if (closeTimerRef.current) window.clearTimeout(closeTimerRef.current);
      if (openFrameRef.current) window.cancelAnimationFrame(openFrameRef.current);
      if (resizeFrameRef.current) window.cancelAnimationFrame(resizeFrameRef.current);
      resizeCleanupRef.current?.();
    };
  }, []);

  const ensureFits = useCallback(async () => {
    const targetWidth = artifactPanelTargetContentWidth({
      isMobile,
      allowWindowResize,
      windowWidth: window.innerWidth,
      splitPaneWidth: splitPaneRef.current?.clientWidth ?? window.innerWidth,
    });
    if (!targetWidth || !window.electron.ensureWindowContentWidth) return;

    await window.electron.ensureWindowContentWidth(targetWidth).catch(() => undefined);
  }, [isMobile, allowWindowResize]);

  /**
   * Rung 2: ask the PANE, not the window, whether the preview panel can have a
   * column.
   *
   * The shipped rule was `isMobile` — window < 930 — which is right for one
   * surface and blind the moment there are two: in a split the pane is decoupled
   * from the window, so a 2-up split in a 1000px window left the panel's 360px
   * floor sitting next to a 140px transcript. isMobile survives inside
   * previewPanelMode as an override, so nothing between 720 and 930 changes.
   */
  const measurePreviewMode = useCallback(() => {
    const paneWidth = splitPaneRef.current?.clientWidth ?? window.innerWidth;
    // Same value → React bails out. This runs from a ResizeObserver, so it must
    // be free at rest.
    setPreviewMode(previewPanelMode({ isMobile, paneWidth }));
  }, [isMobile]);

  const openArtifact = useCallback(
    async (artifact: ArtifactSource) => {
      if (closeTimerRef.current) {
        window.clearTimeout(closeTimerRef.current);
        closeTimerRef.current = null;
      }
      if (openFrameRef.current) {
        window.cancelAnimationFrame(openFrameRef.current);
        openFrameRef.current = null;
      }

      if (!presentedArtifact) {
        widthUserSetRef.current = false;
        await ensureFits();
      }

      // Prime the rung BEFORE the panel exists: the observer below only starts
      // once an artifact is presented, so without this the first frame would
      // paint the panel in whichever mode the last artifact left behind.
      // Measured after ensureFits, which may have grown the OS window.
      measurePreviewMode();
      setPresentedArtifact(artifact);

      if (presentedArtifact) {
        setIsOpen(true);
        return;
      }

      setIsOpen(false);
      openFrameRef.current = window.requestAnimationFrame(() => {
        openFrameRef.current = null;
        setIsOpen(true);
      });
    },
    [ensureFits, measurePreviewMode, presentedArtifact]
  );

  const closePanel = useCallback(() => {
    resizeCleanupRef.current?.();
    setIsResizing(false);
    setIsOpen(false);

    if (closeTimerRef.current) {
      window.clearTimeout(closeTimerRef.current);
    }

    closeTimerRef.current = window.setTimeout(() => {
      closeTimerRef.current = null;
      setPresentedArtifact(null);
      setIsResizing(false);
    }, ARTIFACT_PANEL_EXIT_MS);
  }, []);

  const reset = useCallback(() => {
    resizeCleanupRef.current?.();
    if (closeTimerRef.current) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
    if (openFrameRef.current) {
      window.cancelAnimationFrame(openFrameRef.current);
      openFrameRef.current = null;
    }
    setPresentedArtifact(null);
    setIsOpen(false);
    setIsResizing(false);
    setWidth(null);
    widthUserSetRef.current = false;
  }, []);

  const getMaxWidth = useCallback(() => {
    const containerWidth = splitPaneRef.current?.clientWidth ?? window.innerWidth;
    return Math.max(
      ARTIFACT_PANEL_MIN_WIDTH,
      Math.min(ARTIFACT_PANEL_MAX_WIDTH, containerWidth - ARTIFACT_PANEL_MIN_CHAT_WIDTH)
    );
  }, []);

  const getInitialWidth = useCallback(() => {
    const containerWidth = splitPaneRef.current?.clientWidth ?? window.innerWidth;
    return getDefaultArtifactPanelWidth(containerWidth);
  }, []);

  useEffect(() => {
    if (!presentedArtifact || isMobile || width !== null) return;
    setWidth(getInitialWidth());
  }, [width, getInitialWidth, isMobile, presentedArtifact]);

  useEffect(() => {
    const splitPane = splitPaneRef.current;
    // Gated on `presentedArtifact` only. `isMobile` used to gate this too, but
    // rung 2 has to keep watching the PANE at every window width — a split can
    // starve the transcript in a window the mobile rule calls roomy. The width
    // bookkeeping below stays a no-op in overlay mode, where the panel takes no
    // width from anything.
    if (!splitPane || !presentedArtifact) return;

    const sampleSplitPane = () => {
      measurePreviewMode();
      if (isMobile) return;
      setWidth((currentWidth) => {
        const maxWidth = getMaxWidth();
        if (currentWidth === null || !widthUserSetRef.current) {
          return getInitialWidth();
        }
        const nextWidth = clampArtifactPanelWidth(currentWidth, maxWidth);
        return nextWidth === currentWidth ? currentWidth : nextWidth;
      });
    };

    sampleSplitPane();

    if (typeof ResizeObserver === 'undefined') {
      window.addEventListener('resize', sampleSplitPane);
      return () => window.removeEventListener('resize', sampleSplitPane);
    }

    // No feedback loop: this observes the split pane, which is sized by the
    // layout above it, and rung 2 only ever moves the panel between a column and
    // an overlay INSIDE that pane. Neither mode can change the observed box, so
    // the callback cannot retrigger itself — no hysteresis needed, and none
    // added on spec.
    const resizeObserver = new ResizeObserver(sampleSplitPane);
    resizeObserver.observe(splitPane);
    return () => resizeObserver.disconnect();
  }, [getInitialWidth, getMaxWidth, isMobile, measurePreviewMode, presentedArtifact]);

  const handleResizeStart = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      // A yielded panel has no column to resize. The handle is already unwired at
      // the render site; this is the second lock on the same door.
      if (previewMode === 'overlay') return;

      event.preventDefault();
      event.stopPropagation();

      resizeCleanupRef.current?.();

      const startX = event.clientX;
      const startWidth = width ?? getInitialWidth();
      const previousCursor = document.body.style.cursor;
      const previousUserSelect = document.body.style.userSelect;
      const resizeHandle = event.currentTarget;
      const pointerId = event.pointerId;
      let finished = false;

      try {
        resizeHandle.setPointerCapture(pointerId);
      } catch {
        // Global listeners below still keep the resize usable when capture is unavailable.
      }

      setIsResizing(true);
      widthUserSetRef.current = true;
      pendingWidthRef.current = null;
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';

      const applyPendingWidth = () => {
        resizeFrameRef.current = null;
        const nextWidth = pendingWidthRef.current;
        pendingWidthRef.current = null;
        if (nextWidth !== null) setWidth(nextWidth);
      };

      const scheduleWidth = (nextWidth: number) => {
        pendingWidthRef.current = nextWidth;
        if (resizeFrameRef.current !== null) return;
        resizeFrameRef.current = window.requestAnimationFrame(applyPendingWidth);
      };

      const handleMove = (moveEvent: globalThis.PointerEvent) => {
        if (moveEvent.pointerId !== pointerId) return;
        const nextWidth = startWidth - (moveEvent.clientX - startX);
        scheduleWidth(clampArtifactPanelWidth(nextWidth, getMaxWidth()));
      };

      const finishResize = (commitPendingWidth: boolean, updateState: boolean) => {
        if (finished) return;
        finished = true;
        if (resizeFrameRef.current !== null) {
          window.cancelAnimationFrame(resizeFrameRef.current);
          resizeFrameRef.current = null;
        }
        if (commitPendingWidth && pendingWidthRef.current !== null) {
          setWidth(pendingWidthRef.current);
        }
        pendingWidthRef.current = null;
        if (updateState) setIsResizing(false);
        document.body.style.cursor = previousCursor;
        document.body.style.userSelect = previousUserSelect;
        window.removeEventListener('pointermove', handleMove);
        window.removeEventListener('pointerup', handleEnd);
        window.removeEventListener('pointercancel', handleEnd);
        window.removeEventListener('blur', handleWindowBlur);
        resizeHandle.removeEventListener('lostpointercapture', handleLostPointerCapture);
        try {
          if (resizeHandle.hasPointerCapture(pointerId))
            resizeHandle.releasePointerCapture(pointerId);
        } catch {
          // The element may have left the document while the pointer was outside the window.
        }
        resizeCleanupRef.current = null;
      };

      const handleEnd = (endEvent: globalThis.PointerEvent) => {
        if (endEvent.pointerId !== pointerId) return;
        finishResize(true, true);
      };

      const handleWindowBlur = () => finishResize(true, true);
      const handleLostPointerCapture = (lostEvent: globalThis.PointerEvent) => {
        if (lostEvent.pointerId === pointerId) finishResize(true, true);
      };

      resizeCleanupRef.current = () => finishResize(false, false);

      window.addEventListener('pointermove', handleMove);
      window.addEventListener('pointerup', handleEnd);
      window.addEventListener('pointercancel', handleEnd);
      window.addEventListener('blur', handleWindowBlur);
      resizeHandle.addEventListener('lostpointercapture', handleLostPointerCapture);
    },
    [width, getInitialWidth, getMaxWidth, previewMode]
  );

  const resolvedWidth = width ?? getInitialWidth();

  return {
    splitPaneRef,
    artifact: presentedArtifact,
    previewMode,
    openArtifact,
    closePanel,
    reset,
    viewerProps: {
      artifact: presentedArtifact,
      isOpen,
      isResizing,
      onClose: closePanel,
      onOpenArtifact: openArtifact,
      // Rung 2. 'overlay' means the panel has YIELDED ITS COLUMN: it is
      // absolutely positioned inside the split pane, so it takes no width from
      // the primary column at all and there is no edge to drag. The pane's full
      // width stays the transcript's, and closing the panel reveals it
      // untouched. This is the treatment a mobile-width window has always got;
      // rung 2 only changes WHEN it applies — a pane that cannot seat a 360px
      // transcript beside a 360px panel, at any window size.
      onResizeStart: previewMode === 'overlay' ? undefined : handleResizeStart,
      style:
        previewMode === 'overlay' ? undefined : { width: resolvedWidth, flexBasis: resolvedWidth },
      className:
        previewMode === 'overlay'
          ? 'absolute inset-x-2 bottom-2 top-16 z-[70] rounded-lg border border-border-subtle'
          : 'min-w-[360px] flex-shrink-0',
    },
  };
}
