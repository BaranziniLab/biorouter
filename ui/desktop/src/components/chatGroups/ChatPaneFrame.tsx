import './compact-pane.css';
import { useCallback, useLayoutEffect, useRef, type HTMLAttributes } from 'react';
import type { PaneFitAnswer } from '../Layout/yieldLadder';
import { usePaneFit } from './usePaneFit';

export interface ChatPaneFrameProps extends Omit<
  HTMLAttributes<HTMLDivElement>,
  'children' | 'className' | 'content'
> {
  alone: boolean;
  /** The tab content div and the terminal docks. */
  content: React.ReactNode;
  renderCompact: (state: { answer: PaneFitAnswer; animateIn: boolean }) => React.ReactNode;
  /** ChatDropOverlay, rendered last. */
  overlay?: React.ReactNode;
}

const ACTION_SELECTOR = '[data-compact-pane-action]';

/**
 * One chat pane's box, and rung 5 of the yield ladder: when the split leaves
 * this pane under 360 by 270, it shows the compact placeholder and keeps the
 * chat MOUNTED behind it.
 *
 * The content wrapper is always the root's first child and always rendered, so
 * a change of fit never changes the element type or position above BaseChat,
 * and React never remounts it: the draft, the caret, the scroll, the running
 * turn, the terminal and an open artifact all survive. While compact the
 * wrapper is `inert` (no focus, no hit testing, out of the accessibility tree)
 * and `compact-pane.css` takes it out of the flow at the floor, hidden. Not
 * `aria-hidden`: `inert` already removes it, and Chromium refuses `aria-hidden`
 * on an ancestor of the focused element, which is exactly the moment a pane
 * compacts.
 *
 * Focus follows the person, never the layout: a pane that held focus when it
 * compacted puts it on its placeholder's action, and gives it back to the same
 * element (or the composer) when it returns. A pane that did not hold focus
 * never takes it, so compacting a background pane during a drag never moves the
 * caret in the pane being typed in.
 */
export function ChatPaneFrame({
  alone,
  content,
  renderCompact,
  overlay,
  onScroll,
  ...rest
}: ChatPaneFrameProps) {
  const paneRef = useRef<HTMLDivElement | null>(null);

  /** The element that held focus inside the chat when the pane compacted. */
  const rememberedRef = useRef<HTMLElement | null>(null);
  const focusActionRef = useRef(false);
  const restoreFocusRef = useRef(false);

  // Runs synchronously before the new fit commits, so it sees focus where it
  // was, before `inert` lands on the content or the placeholder unmounts.
  const onWillChange = useCallback((from: PaneFitAnswer, to: PaneFitAnswer) => {
    const pane = paneRef.current;
    if (!pane || from.fit === to.fit) return;
    const active = document.activeElement;
    if (to.fit === 'compact') {
      const chat = pane.querySelector(':scope > .br-chat-pane__content');
      const holds = active instanceof HTMLElement && !!chat?.contains(active);
      rememberedRef.current = holds ? active : null;
      focusActionRef.current = holds;
    } else {
      const compact = pane.querySelector(':scope > .br-compact-pane');
      restoreFocusRef.current = active instanceof HTMLElement && !!compact?.contains(active);
    }
  }, []);

  const { answer, animateIn } = usePaneFit(paneRef, { alone, onWillChange });
  const compact = answer.fit === 'compact';

  useLayoutEffect(() => {
    const pane = paneRef.current;
    if (!pane) return;
    if (compact) {
      if (focusActionRef.current) {
        focusActionRef.current = false;
        pane.querySelector<HTMLElement>(ACTION_SELECTOR)?.focus({ preventScroll: true });
      }
      return;
    }
    // Back to full: the pane scrolls nothing of its own, so put back anything
    // a `scrollIntoView` in the hidden box pushed onto it.
    pane.scrollTop = 0;
    pane.scrollLeft = 0;
    const remembered = rememberedRef.current;
    rememberedRef.current = null;
    if (!restoreFocusRef.current) return;
    restoreFocusRef.current = false;
    if (remembered && remembered.isConnected && !remembered.closest('[inert]')) {
      remembered.focus({ preventScroll: true });
      return;
    }
    pane.querySelector<HTMLElement>('.br-composer-input')?.focus({ preventScroll: true });
  }, [compact]);

  return (
    <div
      {...rest}
      ref={paneRef}
      className="br-chat-pane relative flex min-h-0 min-w-0 flex-1 flex-col"
      data-pane-fit={answer.fit}
      data-pane-shortfall={answer.shortfall ?? undefined}
      onScroll={(event) => {
        onScroll?.(event);
        if (!compact) return;
        const pane = event.currentTarget;
        if (pane.scrollTop !== 0) pane.scrollTop = 0;
        if (pane.scrollLeft !== 0) pane.scrollLeft = 0;
      }}
    >
      <div
        className="br-chat-pane__content flex min-h-0 min-w-0 flex-1 flex-col"
        data-testid="chat-pane-content"
        inert={compact}
      >
        {content}
      </div>
      {compact ? renderCompact({ answer, animateIn }) : null}
      {overlay}
    </div>
  );
}
