import { useCallback, useEffect, useMemo, useRef, type RefObject } from 'react';
import type { ScrollAreaHandle } from '../ui/scroll-area';

/**
 * How far from the end of the content still counts as AT the end. Fractional
 * `scrollTop` under page zoom leaves a sub-pixel gap at a true bottom.
 */
export const TRANSCRIPT_END_SLOP_PX = 2;

interface TranscriptFollowOptions {
  /** The live chat's transcript scroll area. */
  scrollRef: RefObject<ScrollAreaHandle | null>;
  /** `useArtifactPanel`'s `isStacked`: a stacked preview anchors the bottom on its own. */
  isPreviewStacked: () => boolean;
  /** The scroll area is on screen (not the empty state, not a load error). */
  transcriptMounted: boolean;
}

export interface TranscriptFollow {
  /** For `ScrollArea`'s `onScrollChange`: the reader's following state, current at every event. */
  onScrollChange: (atBottom: boolean) => void;
  /** For `ScrollArea`'s `anchorBottomOnResize`. */
  anchorBottomOnResize: () => boolean;
  /**
   * The width change in this frame is not to put the reader back at the end:
   * the chat is keeping a card the person clicked where it was (the side
   * preview's scroll restore), and that is the place to keep.
   */
  keepPlaceForWidthChange: () => void;
}

/**
 * The live chat keeps its newest line in view when its transcript changes size
 * while the reader follows it (vision V2-R6-1, message 1's window scaling).
 *
 * A chat reads bottom-up. `ScrollArea` keeps `scrollTop` when its viewport
 * changes, so anything that shortened the transcript from either edge pushed
 * the newest lines under the composer: the subagent's "Subagent of …" line
 * mounting above the scroller after the open had scrolled to the bottom (32px,
 * every subagent chat), a window made shorter (all of the height it lost), or
 * a narrower window re-wrapping the text taller. Only a stacked preview was
 * anchored, and the rest held by accident when a re-render happened to grow
 * the content.
 *
 * Crew's timeline is the reference and this is its recipe:
 * - HEIGHT. The scroll area's bottom anchor is engaged while a stacked preview
 *   is open (as before) OR while the reader is following. It keeps the line
 *   against the composer where it is, so a reader a little above the end is
 *   not pulled down, and its scroll guard keeps the resize from reading as the
 *   reader scrolling away.
 * - WIDTH. The anchor follows height only, and a width change moves the end
 *   of the content instead (the text re-wraps). A reader who was AT the end
 *   before the change is put back at the end. Unlike Crew's version this asks
 *   "at the end", not "following": following tolerates 200px, and a reader
 *   that far up must not jump when the summary rail or a side preview changes
 *   the column's width. A card the person clicked to open a side preview is
 *   kept where it was instead (`keepPlaceForWidthChange`).
 *
 * A reader who has scrolled up keeps the position they had, for both.
 */
export function useTranscriptFollow({
  scrollRef,
  isPreviewStacked,
  transcriptMounted,
}: TranscriptFollowOptions): TranscriptFollow {
  // ScrollArea starts following, and reports every change through onScrollChange
  // synchronously; a ref, so the predicate reads it between React commits.
  const followingRef = useRef(true);
  const onScrollChange = useCallback((atBottom: boolean) => {
    followingRef.current = atBottom;
  }, []);
  const anchorBottomOnResize = useCallback(
    () => isPreviewStacked() || followingRef.current,
    [isPreviewStacked]
  );
  // Lives until the frame after the one being laid out: the observer runs after
  // that frame's animation callbacks, so a double rAF outlives it.
  const keepPlaceRef = useRef(false);
  const keepPlaceForWidthChange = useCallback(() => {
    keepPlaceRef.current = true;
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        keepPlaceRef.current = false;
      })
    );
  }, []);

  useEffect(() => {
    if (!transcriptMounted) return;
    const viewport = scrollRef.current?.viewportRef.current;
    if (!viewport || typeof ResizeObserver === 'undefined') return;
    const atEnd = () =>
      viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <= TRANSCRIPT_END_SLOP_PX;
    let width = viewport.clientWidth;
    let wasAtEnd = atEnd();
    // The scroll the browser's scroll anchoring makes when a narrower column
    // re-wraps the text reaches this listener AFTER the observer below
    // (measured in Chromium, even with the layout forced in the same task), so
    // the place read here is always the one from before the change.
    const remember = () => {
      wasAtEnd = atEnd();
    };
    const observer = new ResizeObserver(() => {
      const next = viewport.clientWidth;
      if (next !== width) {
        width = next;
        if (wasAtEnd && !keepPlaceRef.current) scrollRef.current?.scrollToBottom('auto');
      }
      wasAtEnd = atEnd();
    });
    viewport.addEventListener('scroll', remember, { passive: true });
    observer.observe(viewport);
    return () => {
      observer.disconnect();
      viewport.removeEventListener('scroll', remember);
    };
  }, [scrollRef, transcriptMounted]);

  // One object for the life of the chat, so a caller can list it as a dependency.
  return useMemo(
    () => ({ onScrollChange, anchorBottomOnResize, keepPlaceForWidthChange }),
    [onScrollChange, anchorBottomOnResize, keepPlaceForWidthChange]
  );
}

/**
 * What the live transcript does each time its list settles
 * (`ProgressiveMessageList`'s `onRenderingComplete`): back to the end while the
 * reader follows it, and nowhere while they have scrolled up.
 *
 * The first settle that has messages lands on the end at once (`'auto'`: a
 * smooth scroll across a whole transcript is a multi-second animation that
 * makes the chat feel slow to open); later ones scroll smoothly.
 *
 * The first settle used to land on the end whatever the reader had done. It is
 * 50ms after the last batch of a long chat lands, and the list opened at the
 * top, so that was the moment the chat first reached its end. A live chat now
 * opens at its end and adds older batches above it (V2-R7-2), so by then the
 * only thing that force did was pull a reader who had scrolled up while the
 * older batches loaded (a few hundred ms on a long chat, seconds on one full
 * of figures) back to the end, off the rows they were reading. A short chat
 * has no batches, so nothing can scroll it before its first settle, and it
 * still lands on the end.
 */
export function settleTranscriptScroll(
  scroll: Pick<ScrollAreaHandle, 'scrollToBottom' | 'isFollowing'> | null | undefined,
  first: boolean
): void {
  if (!scroll?.isFollowing) return;
  if (first) scroll.scrollToBottom('auto');
  else scroll.scrollToBottom();
}

/**
 * BaseChat's `onRenderingComplete`: `settleTranscriptScroll`, with the first
 * settle counted once, at the first one that has messages (a chat's messages
 * arrive after it mounts).
 */
export function useTranscriptSettle(
  scrollRef: RefObject<ScrollAreaHandle | null>,
  messageCount: number
): () => void {
  const firstRef = useRef(true);
  return useCallback(() => {
    const first = firstRef.current && messageCount > 0;
    if (first) firstRef.current = false;
    settleTranscriptScroll(scrollRef.current, first);
  }, [scrollRef, messageCount]);
}
