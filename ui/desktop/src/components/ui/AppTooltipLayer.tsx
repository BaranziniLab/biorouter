import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { cn } from '../../utils';
import { TOOLTIP_SURFACE_CLASS_NAME } from './Tooltip';

const TOOLTIP_ATTRIBUTE = 'data-biorouter-tooltip';
const GENERATED_ARIA_LABEL_ATTRIBUTE = 'data-biorouter-tooltip-aria-label';
const TOOLTIP_SELECTOR = `[${TOOLTIP_ATTRIBUTE}]`;
const TOOLTIP_OPEN_DELAY_MS = 500;
const TOOLTIP_OFFSET = 8;
const VIEWPORT_PADDING = 8;
const INTERACTIVE_SELECTOR =
  'button, a[href], input, select, textarea, iframe, [role="button"], [role="menuitem"], [role="menuitemcheckbox"], [role="menuitemradio"]';

interface TooltipState {
  target: HTMLElement;
  text: string;
  x: number;
  y: number;
  side: 'top' | 'bottom';
}

function upgradeNativeTitle(element: HTMLElement) {
  const rawTitle = element.getAttribute('title');
  if (rawTitle === null) {
    element.removeAttribute(TOOLTIP_ATTRIBUTE);
    if (element.hasAttribute(GENERATED_ARIA_LABEL_ATTRIBUTE)) {
      element.removeAttribute('aria-label');
      element.removeAttribute(GENERATED_ARIA_LABEL_ATTRIBUTE);
    }
    return;
  }

  const title = rawTitle.trim();
  if (!title) return;

  element.setAttribute(TOOLTIP_ATTRIBUTE, title);
  element.setAttribute('title', '');

  if (
    element.matches(INTERACTIVE_SELECTOR) &&
    (element.hasAttribute(GENERATED_ARIA_LABEL_ATTRIBUTE) ||
      (!element.hasAttribute('aria-label') && !element.hasAttribute('aria-labelledby')))
  ) {
    element.setAttribute('aria-label', title);
    element.setAttribute(GENERATED_ARIA_LABEL_ATTRIBUTE, '');
  }
}

function upgradeTitlesWithin(root: HTMLElement) {
  if (root.hasAttribute('title')) upgradeNativeTitle(root);
  root.querySelectorAll<HTMLElement>('[title]').forEach(upgradeNativeTitle);
}

function tooltipTargetFromEvent(event: Event): HTMLElement | null {
  return event.target instanceof Element
    ? event.target.closest<HTMLElement>(TOOLTIP_SELECTOR)
    : null;
}

export function AppTooltipLayer() {
  const [tooltip, setTooltip] = useState<TooltipState | null>(null);
  const [horizontalShift, setHorizontalShift] = useState(0);
  const tooltipRef = useRef<HTMLDivElement>(null);
  const openTimerRef = useRef<number | null>(null);

  useEffect(() => {
    upgradeTitlesWithin(document.body);

    const observer = new MutationObserver((records) => {
      records.forEach((record) => {
        if (record.type === 'attributes' && record.target instanceof HTMLElement) {
          upgradeNativeTitle(record.target);
          return;
        }

        record.addedNodes.forEach((node) => {
          if (node instanceof HTMLElement) upgradeTitlesWithin(node);
        });
      });
    });

    observer.observe(document.body, {
      attributeFilter: ['title'],
      attributes: true,
      childList: true,
      subtree: true,
    });

    const showTooltip = (event: Event) => {
      const target = tooltipTargetFromEvent(event);
      const text = target?.getAttribute(TOOLTIP_ATTRIBUTE)?.trim();
      if (!target || !text) return;

      if (event.type === 'pointerover') {
        const relatedTarget = (event as PointerEvent).relatedTarget;
        const previousTarget =
          relatedTarget instanceof Element
            ? relatedTarget.closest<HTMLElement>(TOOLTIP_SELECTOR)
            : null;
        if (previousTarget === target) return;
      }

      if (openTimerRef.current !== null) window.clearTimeout(openTimerRef.current);
      openTimerRef.current = window.setTimeout(() => {
        openTimerRef.current = null;
        if (!target.isConnected) return;

        const bounds = target.getBoundingClientRect();
        const spaceAbove = bounds.top - TOOLTIP_OFFSET - VIEWPORT_PADDING;
        const spaceBelow = window.innerHeight - bounds.bottom - TOOLTIP_OFFSET - VIEWPORT_PADDING;
        const side = spaceAbove >= spaceBelow ? 'top' : 'bottom';
        setHorizontalShift(0);
        setTooltip({
          target,
          text,
          x: bounds.left + bounds.width / 2,
          y: side === 'top' ? bounds.top - TOOLTIP_OFFSET : bounds.bottom + TOOLTIP_OFFSET,
          side,
        });
      }, TOOLTIP_OPEN_DELAY_MS);
    };

    const hideTooltip = (event?: Event) => {
      if (event?.type === 'pointerout') {
        const currentTarget = tooltipTargetFromEvent(event);
        const relatedTarget = (event as PointerEvent).relatedTarget;
        const nextTarget =
          relatedTarget instanceof Element
            ? relatedTarget.closest<HTMLElement>(TOOLTIP_SELECTOR)
            : null;
        if (currentTarget && currentTarget === nextTarget) return;
      }
      if (openTimerRef.current !== null) {
        window.clearTimeout(openTimerRef.current);
        openTimerRef.current = null;
      }
      setTooltip(null);
    };

    const hideOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') hideTooltip();
    };

    document.addEventListener('pointerover', showTooltip, true);
    document.addEventListener('pointerout', hideTooltip, true);
    document.addEventListener('focusin', showTooltip, true);
    document.addEventListener('focusout', hideTooltip, true);
    document.addEventListener('pointerdown', hideTooltip, true);
    document.addEventListener('keydown', hideOnEscape, true);
    window.addEventListener('blur', hideTooltip);
    window.addEventListener('resize', hideTooltip);
    window.addEventListener('scroll', hideTooltip, true);

    return () => {
      if (openTimerRef.current !== null) window.clearTimeout(openTimerRef.current);
      observer.disconnect();
      document.removeEventListener('pointerover', showTooltip, true);
      document.removeEventListener('pointerout', hideTooltip, true);
      document.removeEventListener('focusin', showTooltip, true);
      document.removeEventListener('focusout', hideTooltip, true);
      document.removeEventListener('pointerdown', hideTooltip, true);
      document.removeEventListener('keydown', hideOnEscape, true);
      window.removeEventListener('blur', hideTooltip);
      window.removeEventListener('resize', hideTooltip);
      window.removeEventListener('scroll', hideTooltip, true);
    };
  }, []);

  useLayoutEffect(() => {
    const surface = tooltipRef.current;
    if (!surface || !tooltip) return;

    const surfaceBounds = surface.getBoundingClientRect();
    const targetBounds = tooltip.target.getBoundingClientRect();
    const spaceAbove = targetBounds.top - TOOLTIP_OFFSET - VIEWPORT_PADDING;
    const spaceBelow = window.innerHeight - targetBounds.bottom - TOOLTIP_OFFSET - VIEWPORT_PADDING;
    const side = spaceAbove >= surfaceBounds.height || spaceAbove >= spaceBelow ? 'top' : 'bottom';
    const x = targetBounds.left + targetBounds.width / 2;
    const y =
      side === 'top' ? targetBounds.top - TOOLTIP_OFFSET : targetBounds.bottom + TOOLTIP_OFFSET;
    const unshiftedLeft = x - surfaceBounds.width / 2;
    const unshiftedRight = x + surfaceBounds.width / 2;
    let nextShift = 0;
    if (unshiftedLeft < VIEWPORT_PADDING) {
      nextShift = VIEWPORT_PADDING - unshiftedLeft;
    } else if (unshiftedRight > window.innerWidth - VIEWPORT_PADDING) {
      nextShift = window.innerWidth - VIEWPORT_PADDING - unshiftedRight;
    }

    if (nextShift !== horizontalShift) setHorizontalShift(nextShift);
    if (side !== tooltip.side || x !== tooltip.x || y !== tooltip.y) {
      setTooltip((current) =>
        current?.target === tooltip.target ? { ...current, side, x, y } : current
      );
    }
  }, [horizontalShift, tooltip]);

  // A tooltip outlives its target when the target LEAVES rather than when the
  // pointer does: hover a row control, change route without moving the pointer,
  // and no `pointerout` is ever delivered — the element the pointer was over is
  // simply gone, and the tooltip stays on screen describing nothing.
  //
  // ⚠ This used to be `useEffect(… , [tooltip])`, which is a check that runs
  // only when the tooltip STATE changes — never for the one case it was written
  // for. Adding `tooltip.target` to the deps does not fix it either: a node
  // reference does not change when the node is removed. The removal is a DOM
  // event, so it takes a DOM observer.
  const tooltipTarget = tooltip?.target ?? null;
  useEffect(() => {
    if (!tooltipTarget) return;
    if (!tooltipTarget.isConnected) {
      setTooltip(null);
      return;
    }
    const dismissWhenTargetLeaves = () => {
      if (!tooltipTarget.isConnected) setTooltip(null);
    };
    // Only while a tooltip is open, and only an `isConnected` read per batch —
    // the observer is torn down the moment the tooltip closes.
    const observer = new MutationObserver(dismissWhenTargetLeaves);
    observer.observe(document.body, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, [tooltipTarget]);

  if (!tooltip) return null;

  return createPortal(
    <div
      ref={tooltipRef}
      role="tooltip"
      data-slot="tooltip-content"
      data-side={tooltip.side}
      className={cn(
        TOOLTIP_SURFACE_CLASS_NAME,
        'pointer-events-none fixed max-h-[calc(100vh-16px)] overflow-hidden whitespace-pre-line animate-in fade-in-0 duration-[120ms]'
      )}
      style={{
        left: tooltip.x + horizontalShift,
        top: tooltip.y,
        transform: tooltip.side === 'top' ? 'translate(-50%, -100%)' : 'translate(-50%, 0)',
      }}
    >
      {tooltip.text}
    </div>,
    document.body
  );
}
