import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties, PointerEvent as ReactPointerEvent, RefObject } from 'react';
import {
  PREVIEW_MIN_WIDTH,
  PREVIEW_PREFERRED_CHAT_WIDTH,
  previewChatFloor,
  previewPanelMode,
  previewSideWidth,
  previewStackDrag,
  previewStackHeight,
  type PreviewPanelMode,
} from '../Layout/yieldLadder';
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
 *
 * RUNG 2 OF THE YIELD LADDER (the geometry). Every decision is a pure function in
 * `Layout/yieldLadder.ts`; this hook only measures the split box and hands the
 * answers to CSS:
 *
 *   - the split box gets `data-preview-split`, `data-preview-layout` (side |
 *     stack), `data-preview-folded`, and the lengths `--preview-panel-width`,
 *     `--preview-stack-height` and `--preview-chat-floor`;
 *   - ONE authored grid in `main.css` places the header, the preview, the
 *     transcript and the composer by `data-preview-area`, so side ↔ stack is a
 *     template change on the same DOM nodes. Nothing is re-parented and no
 *     `<ArtifactViewer>` is re-keyed, which is what keeps a figure's frame from
 *     reloading at every crossing.
 */

// The floor is shared with the ladder rather than re-declared, so the number the
// rule reasons about and the number the panel clamps to cannot drift apart.
const ARTIFACT_PANEL_MIN_WIDTH = PREVIEW_MIN_WIDTH;
// The conversation's PREFERRED width beside the panel. The window still grows to
// seat a 360px panel beside it plus the composer gutter: unchanged by rung 2.
const ARTIFACT_PANEL_MIN_CHAT_WIDTH = PREVIEW_PREFERRED_CHAT_WIDTH;
const ARTIFACT_PANEL_AUTO_TUCK_WIDTH =
  ARTIFACT_PANEL_MIN_WIDTH + ARTIFACT_PANEL_MIN_CHAT_WIDTH + 48;
const ARTIFACT_PANEL_AUTO_EXPAND_PADDING = 24;
// Matches the panel's close transition (--motion-fast); exit is a tier faster
// than the --motion-base entrance so the panel unmounts as the slide completes.
const ARTIFACT_PANEL_EXIT_MS = 125;
/**
 * How long a freshly opened sheet waits, invisible and taking no room, for its
 * content to say how tall it is. Text answers in the same frame it renders; a
 * figure answers when its frame has laid out. A document that never answers
 * (a broken figure, a slow read) gets the default half after this.
 */
export const ARTIFACT_PANEL_MEASURE_TIMEOUT_MS = 600;

/** The split box a host spreads `splitPaneProps` onto. */
export const PREVIEW_SPLIT_ATTR = 'data-preview-split';
/**
 * Where an element sits in the rung-2 grid: `column` and `body` are flattened
 * (`display: contents`), `header`, `subheader`, `transcript` and `composer` are
 * placed. A replay marks its whole reading column `conversation`.
 */
export const PREVIEW_AREA_ATTR = 'data-preview-area';
/** The box whose height is the transcript, for measuring the conversation's chrome. */
export const PREVIEW_TRANSCRIPT_ATTR = 'data-preview-transcript';

export function getDefaultArtifactPanelWidth(containerWidth: number): number {
  return previewSideWidth({ paneWidth: containerWidth });
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

/** What rung 2 reads off the split box. */
export interface PreviewPaneGeometry {
  /** The split box's width — the pane, never the window. */
  paneWidth: number;
  /** The split box's height below its header band (H). */
  bodyHeight: number;
  /** Everything in the conversation that is not transcript (the composer bar), or null. */
  chrome: number | null;
}

const UNMEASURED: PreviewPaneGeometry = { paneWidth: 0, bodyHeight: 0, chrome: null };

/**
 * Read rung 2's inputs off a split box. Every term is independent of the sheet's
 * own height, so resizing the sheet to the answer cannot change the answer:
 *
 *   - H is the box's height less its header band (the chat header and a
 *     subagent's second header, or nothing in a replay);
 *   - the chrome is H less the row a stacked sheet occupies (the gap between the
 *     header band and the conversation's top) less the transcript's box — the
 *     composer bar in a chat, the page header in a replay. It is MEASURED, not a
 *     constant, so a composer holding a queued message or a long draft counts.
 */
export function measurePreviewPaneGeometry(split: HTMLElement): PreviewPaneGeometry {
  const splitTop = split.getBoundingClientRect().top;
  let headerBottom = 0;
  for (const band of split.querySelectorAll<HTMLElement>(
    `[${PREVIEW_AREA_ATTR}="header"], [${PREVIEW_AREA_ATTR}="subheader"]`
  )) {
    const rect = band.getBoundingClientRect();
    if (rect.height > 0) headerBottom = Math.max(headerBottom, rect.bottom - splitTop);
  }
  const bodyHeight = Math.max(0, Math.round(split.clientHeight - headerBottom));

  let chrome: number | null = null;
  const conversation = split.querySelector<HTMLElement>(
    `[${PREVIEW_AREA_ATTR}="transcript"], [${PREVIEW_AREA_ATTR}="conversation"]`
  );
  const transcript = split.querySelector<HTMLElement>(`[${PREVIEW_TRANSCRIPT_ATTR}]`);
  if (conversation && transcript) {
    const sheetRow = Math.max(
      0,
      conversation.getBoundingClientRect().top - splitTop - headerBottom
    );
    const value = bodyHeight - sheetRow - transcript.getBoundingClientRect().height;
    if (Number.isFinite(value) && value >= 0) chrome = Math.round(value);
  }
  return { paneWidth: Math.round(split.clientWidth), bodyHeight, chrome };
}

export interface UseArtifactPanelOptions {
  /**
   * A mobile-width window. Callers pass `useIsMobile()`. It does NOT decide where
   * the panel goes — the pane's own width does, at every window width — it only
   * stops a phone-sized surface from trying to grow its window.
   */
  isMobile: boolean;
  /**
   * Grow the OS window to seat the panel when the pane is too narrow. Only the
   * focused chat may do this; read-only transcripts pass false.
   */
  allowWindowResize?: boolean;
  /**
   * Whether the host actually renders the panel it holds. A chat in a background
   * group keeps its artifact but draws no panel, and its split box must then keep
   * the ordinary layout. Defaults to true.
   */
  enabled?: boolean;
}

/** Everything a host spreads onto `<ArtifactViewer>` so the three mounts cannot drift. */
export interface ArtifactViewerHostProps {
  artifact: ArtifactSource | null;
  isOpen: boolean;
  motionReady: boolean;
  isResizing: boolean;
  onClose: () => void;
  onOpenArtifact: (artifact: ArtifactSource) => void;
  onResizeStart: ((event: ReactPointerEvent<HTMLDivElement>) => void) | undefined;
  layout: PreviewPanelMode;
  folded: boolean;
  onToggleFold: () => void;
  onUnfold: () => void;
  onContentHeightChange: (height: number | null) => void;
}

/** Spread onto the split box: rung 2's attributes and lengths, read by `main.css`. */
export interface PreviewSplitPaneProps {
  'data-preview-split': '';
  'data-preview-layout'?: PreviewPanelMode;
  'data-preview-folded'?: '';
  'data-preview-measuring'?: '';
  style?: CSSProperties;
}

export interface ArtifactPanelController {
  /**
   * Attach to the element that is the split container — the box the
   * conversation and the panel share. Rung 2 measures THIS, not the window: in
   * a split pane the two are decoupled.
   */
  splitPaneRef: RefObject<HTMLDivElement | null>;
  splitPaneProps: PreviewSplitPaneProps;
  /** Null when no artifact is presented — the host skips the mount entirely. */
  artifact: ArtifactSource | null;
  previewMode: PreviewPanelMode;
  /**
   * Whether a stacked sheet is on screen right now. Read at the moment a
   * transcript's viewport resizes (`ScrollArea`'s `anchorBottomOnResize`), so it
   * is a function over a ref rather than a value captured at render.
   */
  isStacked: () => boolean;
  openArtifact: (artifact: ArtifactSource) => Promise<void>;
  closePanel: () => void;
  /** Tear the panel down without animating — for an identity change (new session). */
  reset: () => void;
  viewerProps: ArtifactViewerHostProps;
}

type StackDrag = { folded: boolean; height: number };

export function useArtifactPanel(options: UseArtifactPanelOptions): ArtifactPanelController {
  const { isMobile, allowWindowResize = false, enabled = true } = options;

  const [presentedArtifact, setPresentedArtifact] = useState<ArtifactSource | null>(null);
  const [isOpen, setIsOpen] = useState(false);
  const [isResizing, setIsResizing] = useState(false);
  // The width the user dragged a SIDE panel to; null = the ladder's default.
  const [userWidth, setUserWidth] = useState<number | null>(null);
  // Rung 2's inputs, re-read from ResizeObserver callbacks that bail out on equal
  // values — so a splitter drag that does not change the answer re-renders nothing.
  const [geometry, setGeometry] = useState<PreviewPaneGeometry>(UNMEASURED);
  // The share of the body a user dragged a STACKED sheet to; null = never dragged.
  const [stackRatio, setStackRatio] = useState<number | null>(null);
  const [folded, setFolded] = useState(false);
  // How tall the sheet's content needs it to be (strip included), as the panel
  // measured it; null = the content cannot say.
  const [contentHeight, setContentHeight] = useState<number | null>(null);
  // A fresh sheet takes no room until its content has answered (see main.css).
  const [measuring, setMeasuring] = useState(false);
  const [stackDrag, setStackDrag] = useState<StackDrag | null>(null);

  const splitPaneRef = useRef<HTMLDivElement>(null);
  const openRequestRef = useRef(0);
  const closeTimerRef = useRef<number | null>(null);
  const openFrameRef = useRef<number | null>(null);
  const measureTimerRef = useRef<number | null>(null);
  const resizeFrameRef = useRef<number | null>(null);
  const pendingSizeRef = useRef<(() => void) | null>(null);
  const resizeCleanupRef = useRef<(() => void) | null>(null);

  const [previewMode, setPreviewMode] = useState<PreviewPanelMode>('side');
  const mounted = Boolean(presentedArtifact && enabled);
  const stackedRef = useRef(false);
  stackedRef.current = mounted && previewMode === 'stack';

  useEffect(() => {
    return () => {
      openRequestRef.current += 1;
      if (closeTimerRef.current) window.clearTimeout(closeTimerRef.current);
      if (openFrameRef.current) window.cancelAnimationFrame(openFrameRef.current);
      if (measureTimerRef.current) window.clearTimeout(measureTimerRef.current);
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

  /** Re-read the split box. Same values → React bails out, so it is free at rest. */
  const measureGeometry = useCallback(() => {
    const split = splitPaneRef.current;
    if (!split) return;
    const next = measurePreviewPaneGeometry(split);
    setPreviewMode((previous) => previewPanelMode({ paneWidth: next.paneWidth, previous }));
    setGeometry((previous) =>
      previous.paneWidth === next.paneWidth &&
      previous.bodyHeight === next.bodyHeight &&
      previous.chrome === next.chrome
        ? previous
        : next
    );
  }, []);

  const stopMeasuring = useCallback(() => {
    if (measureTimerRef.current) {
      window.clearTimeout(measureTimerRef.current);
      measureTimerRef.current = null;
    }
    setMeasuring(false);
  }, []);

  const openArtifact = useCallback(
    async (artifact: ArtifactSource) => {
      const request = ++openRequestRef.current;
      if (closeTimerRef.current) {
        window.clearTimeout(closeTimerRef.current);
        closeTimerRef.current = null;
      }
      if (openFrameRef.current) {
        window.cancelAnimationFrame(openFrameRef.current);
        openFrameRef.current = null;
      }

      if (!presentedArtifact) {
        setUserWidth(null);
        setStackRatio(null);
        setFolded(false);
        setContentHeight(null);
        await ensureFits();
      }
      if (request !== openRequestRef.current) return;

      // Prime rung 2 BEFORE the panel exists: the observer below only starts once
      // an artifact is presented, so without this the first frame would paint the
      // panel in whichever shape the last artifact left behind. Measured after
      // ensureFits, which may have grown the OS window.
      measureGeometry();
      setPresentedArtifact(artifact);

      if (presentedArtifact) {
        setIsOpen(true);
        return;
      }

      // A fresh sheet waits for its content's height before it takes any room,
      // so the conversation moves once — to the right height — instead of to half
      // and then to the content (the flash a prototype measured at 33ms → 101ms).
      setMeasuring(true);
      if (measureTimerRef.current) window.clearTimeout(measureTimerRef.current);
      measureTimerRef.current = window.setTimeout(() => {
        measureTimerRef.current = null;
        setMeasuring(false);
      }, ARTIFACT_PANEL_MEASURE_TIMEOUT_MS);

      setIsOpen(false);
      openFrameRef.current = window.requestAnimationFrame(() => {
        openFrameRef.current = null;
        setIsOpen(true);
      });
    },
    [ensureFits, measureGeometry, presentedArtifact]
  );

  const closePanel = useCallback(() => {
    openRequestRef.current += 1;
    if (openFrameRef.current) {
      window.cancelAnimationFrame(openFrameRef.current);
      openFrameRef.current = null;
    }
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
      setStackDrag(null);
      stopMeasuring();
    }, ARTIFACT_PANEL_EXIT_MS);
  }, [stopMeasuring]);

  const reset = useCallback(() => {
    openRequestRef.current += 1;
    resizeCleanupRef.current?.();
    if (closeTimerRef.current) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
    if (openFrameRef.current) {
      window.cancelAnimationFrame(openFrameRef.current);
      openFrameRef.current = null;
    }
    stopMeasuring();
    setPresentedArtifact(null);
    setIsOpen(false);
    setIsResizing(false);
    setUserWidth(null);
    setStackRatio(null);
    setFolded(false);
    setContentHeight(null);
    setStackDrag(null);
  }, [stopMeasuring]);

  const handleContentHeightChange = useCallback(
    (height: number | null) => {
      setContentHeight((previous) => (previous === height ? previous : height));
      stopMeasuring();
    },
    [stopMeasuring]
  );

  const toggleFold = useCallback(() => setFolded((value) => !value), []);
  const unfold = useCallback(() => setFolded(false), []);

  useEffect(() => {
    const splitPane = splitPaneRef.current;
    if (!splitPane || !presentedArtifact) return;

    measureGeometry();

    if (typeof ResizeObserver === 'undefined') {
      window.addEventListener('resize', measureGeometry);
      return () => window.removeEventListener('resize', measureGeometry);
    }

    // No feedback loop: this observes the split box, which is sized by the layout
    // above it, and rung 2 only ever moves the panel INSIDE that box. Neither
    // shape can change the observed box. The return buffer prevents repeated
    // flips during a window-edge drag. The header bands are watched
    // because a subagent's second header loading in moves H without resizing the
    // box; the composer because its growth is the conversation's chrome.
    const resizeObserver = new ResizeObserver(measureGeometry);
    resizeObserver.observe(splitPane);
    for (const element of splitPane.querySelectorAll(
      `[${PREVIEW_AREA_ATTR}="header"], [${PREVIEW_AREA_ATTR}="subheader"], [${PREVIEW_AREA_ATTR}="composer"]`
    )) {
      resizeObserver.observe(element);
    }
    return () => resizeObserver.disconnect();
  }, [measureGeometry, presentedArtifact]);

  const { paneWidth, bodyHeight, chrome } = geometry;
  const chatFloor = previewChatFloor(chrome);
  const resolvedWidth = previewSideWidth({ paneWidth, userWidth });
  const settledStackHeight = previewStackHeight({
    bodyHeight,
    chatFloor,
    ratio: stackRatio,
    contentHeight,
    folded,
  });
  const stackHeight = stackDrag ? stackDrag.height : measuring ? 0 : settledStackHeight;
  // The height a still-measuring sheet is laid out at while it takes no room: the
  // undragged default, so a figure's frame has a real viewport to lay out in.
  const provisionalStackHeight = previewStackHeight({ bodyHeight, chatFloor });

  const handleResizeStart = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      // One gesture, two axes: a side column drags its left edge, a stacked sheet
      // its bottom edge. Both clamp through the ladder's own functions, so a drag
      // can never reach a size the automatic sizing would refuse.
      const axis = previewMode === 'stack' ? 'y' : 'x';

      event.preventDefault();
      event.stopPropagation();

      resizeCleanupRef.current?.();

      const startX = event.clientX;
      const startY = event.clientY;
      const startWidth = resolvedWidth;
      const startStackHeight = settledStackHeight;
      const split = splitPaneRef.current;
      const startGeometry = split ? measurePreviewPaneGeometry(split) : geometry;
      const startFloor = previewChatFloor(startGeometry.chrome);
      const previousCursor = document.body.style.cursor;
      const previousUserSelect = document.body.style.userSelect;
      const resizeHandle = event.currentTarget;
      const pointerId = event.pointerId;
      let finished = false;
      let latestDrag: ReturnType<typeof previewStackDrag> | null = null;
      let latestWidth: number | null = null;

      try {
        resizeHandle.setPointerCapture(pointerId);
      } catch {
        // Global listeners below still keep the resize usable when capture is unavailable.
      }

      setIsResizing(true);
      document.body.style.cursor = axis === 'y' ? 'row-resize' : 'col-resize';
      document.body.style.userSelect = 'none';

      const applyPending = () => {
        resizeFrameRef.current = null;
        const apply = pendingSizeRef.current;
        pendingSizeRef.current = null;
        apply?.();
      };

      const schedule = (apply: () => void) => {
        pendingSizeRef.current = apply;
        if (resizeFrameRef.current !== null) return;
        resizeFrameRef.current = window.requestAnimationFrame(applyPending);
      };

      const handleMove = (moveEvent: globalThis.PointerEvent) => {
        if (moveEvent.pointerId !== pointerId) return;
        if (axis === 'y') {
          const drag = previewStackDrag({
            bodyHeight: startGeometry.bodyHeight,
            chatFloor: startFloor,
            wantedHeight: startStackHeight + (moveEvent.clientY - startY),
          });
          latestDrag = drag;
          schedule(() => setStackDrag({ folded: drag.folded, height: drag.height }));
          return;
        }
        const next = previewSideWidth({
          paneWidth: splitPaneRef.current?.clientWidth ?? startGeometry.paneWidth,
          userWidth: startWidth - (moveEvent.clientX - startX),
        });
        latestWidth = next;
        schedule(() => setUserWidth(next));
      };

      const finishResize = (commit: boolean, updateState: boolean) => {
        if (finished) return;
        finished = true;
        if (resizeFrameRef.current !== null) {
          window.cancelAnimationFrame(resizeFrameRef.current);
          resizeFrameRef.current = null;
        }
        pendingSizeRef.current = null;
        if (commit && axis === 'x' && latestWidth !== null) setUserWidth(latestWidth);
        if (commit && axis === 'y' && latestDrag !== null) {
          // A drag that folds keeps the size the sheet had when the drag began:
          // unfolding returns to it, not to the sliver the pointer passed through.
          if (latestDrag.folded) setFolded(true);
          else {
            setFolded(false);
            setStackRatio(latestDrag.ratio);
          }
        }
        if (updateState) {
          setIsResizing(false);
          setStackDrag(null);
        }
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

      resizeCleanupRef.current = () => {
        finishResize(false, false);
        setStackDrag(null);
      };

      window.addEventListener('pointermove', handleMove);
      window.addEventListener('pointerup', handleEnd);
      window.addEventListener('pointercancel', handleEnd);
      window.addEventListener('blur', handleWindowBlur);
      resizeHandle.addEventListener('lostpointercapture', handleLostPointerCapture);
    },
    [geometry, previewMode, resolvedWidth, settledStackHeight]
  );

  const isStacked = useCallback(() => stackedRef.current, []);

  const splitPaneProps: PreviewSplitPaneProps = mounted
    ? {
        [PREVIEW_SPLIT_ATTR]: '',
        'data-preview-layout': previewMode,
        'data-preview-folded': folded && !stackDrag ? '' : undefined,
        'data-preview-measuring': measuring && !stackDrag ? '' : undefined,
        style: {
          '--preview-panel-width': `${resolvedWidth}px`,
          '--preview-stack-height': `${stackHeight}px`,
          '--preview-provisional-height': `${provisionalStackHeight}px`,
          '--preview-chat-floor': `${chatFloor}px`,
        } as CSSProperties,
      }
    : { [PREVIEW_SPLIT_ATTR]: '' };

  return {
    splitPaneRef,
    splitPaneProps,
    artifact: presentedArtifact,
    previewMode,
    isStacked,
    openArtifact,
    closePanel,
    reset,
    viewerProps: {
      artifact: presentedArtifact,
      isOpen,
      motionReady: previewMode === 'side' || !measuring,
      isResizing,
      onClose: closePanel,
      onOpenArtifact: openArtifact,
      onResizeStart: handleResizeStart,
      layout: previewMode,
      folded: stackDrag ? stackDrag.folded : folded,
      onToggleFold: toggleFold,
      onUnfold: unfold,
      onContentHeightChange: handleContentHeightChange,
    },
  };
}
