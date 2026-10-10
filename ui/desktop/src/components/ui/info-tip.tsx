'use client';

import * as React from 'react';
import * as PopoverPrimitive from '@radix-ui/react-popover';
import { Slot } from '@radix-ui/react-slot';

import { cn } from '../../utils';
import { Info } from '../icons/app-icons';
import { uiCopy } from './copy';
import { TOOLTIP_SURFACE_CLASS_NAME, isFocusFromTabKey } from './Tooltip';

/**
 * InfoTip: the one help affordance (spec 2.6, principle 2 "say it once, then hide the
 * explanation").
 *
 * A visible 14px info glyph in subtle ink sits 4px after a label. Its explanation opens:
 *   - on pointer hover after 200ms,
 *   - at once when the Tab key moves focus onto it (Tooltip's gate, so a focus a program
 *     restores never opens it),
 *   - on click or tap, as a toggle (a click while a hover has it open pins it instead).
 * It closes on pointer leave (100ms grace, enough to cross the 8px gap), blur, Escape and
 * scroll.
 *
 * It is never hover-only: the same text is always rendered in a visually hidden node, the
 * trigger points at it with `aria-describedby`, and `useInfoTipId()` / `id` let the control it
 * explains point at the same node, so a screen reader hears the help on reaching the control.
 *
 * Rules (spec 2.6):
 *   - plain text, two sentences at most; no links, buttons or bold runs (inline ink vanishes on
 *     the inverse ground). Anything longer belongs in a `Disclosure` or a `Popover`;
 *   - never `title=` (AppTooltipLayer turns a title into the control's name);
 *   - never inside a `<label>` or a row whose click toggles a control: it sits as the label's
 *     next sibling, and a row's click-to-toggle handler ignores events from `.br-info-tip`;
 *   - never the only carrier of an error, a refusal, a privacy disclosure or a destructive
 *     consequence.
 *
 * Built on the Popover primitive's anchor and content rather than on Radix Tooltip, because a
 * Radix Tooltip closes on every click of its trigger and so cannot toggle.
 */

export const INFO_TIP_OPEN_DELAY_MS = 200;
export const INFO_TIP_CLOSE_GRACE_MS = 100;

/** A stable id for an InfoTip's description, to share with the control it explains. */
export function useInfoTipId(): string {
  const reactId = React.useId();
  return `br-info-tip-${reactId.replace(/[^a-zA-Z0-9_-]/g, '')}`;
}

export interface InfoTipProps {
  /**
   * What the help is about. The glyph's accessible name is "About {label}". In the `asChild`
   * form the child keeps its own name and `label` is not spoken.
   */
  label: string;
  /** The help: plain text, at most two sentences (about 160 characters). */
  help?: string;
  /**
   * Without `asChild`: the help text, when `help` is not given.
   * With `asChild`: the element that becomes the focusable trigger (a Badge, a chip).
   */
  children?: React.ReactNode;
  /** The id of the hidden description node. Give the explained control `aria-describedby={id}`. */
  id?: string;
  /** Make `children` the trigger instead of drawing the glyph (Crew's ClassificationBadge). */
  asChild?: boolean;
  side?: 'top' | 'right' | 'bottom' | 'left';
  align?: 'start' | 'center' | 'end';
  className?: string;
  'data-testid'?: string;
}

type OpenState = { open: boolean; pinned: boolean };
const CLOSED: OpenState = { open: false, pinned: false };

export const InfoTip = React.forwardRef<HTMLElement, InfoTipProps>(function InfoTip(
  {
    label,
    help,
    children,
    id,
    asChild = false,
    side = 'top',
    align = 'start',
    className,
    'data-testid': testId,
  },
  forwardedRef
) {
  const generatedId = useInfoTipId();
  const descriptionId = id ?? generatedId;
  const text = help ?? (asChild ? undefined : children);
  const triggerRef = React.useRef<HTMLElement | null>(null);
  const [state, setState] = React.useState<OpenState>(CLOSED);
  const openTimer = React.useRef<number | null>(null);
  const closeTimer = React.useRef<number | null>(null);

  const setRefs = React.useCallback(
    (node: HTMLElement | null) => {
      triggerRef.current = node;
      if (typeof forwardedRef === 'function') forwardedRef(node);
      else if (forwardedRef) forwardedRef.current = node;
    },
    [forwardedRef]
  );

  const clearTimers = React.useCallback(() => {
    if (openTimer.current !== null) window.clearTimeout(openTimer.current);
    if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    openTimer.current = null;
    closeTimer.current = null;
  }, []);

  React.useEffect(() => clearTimers, [clearTimers]);

  const close = React.useCallback(() => {
    clearTimers();
    setState(CLOSED);
  }, [clearTimers]);

  const scheduleClose = React.useCallback(() => {
    if (openTimer.current !== null) {
      window.clearTimeout(openTimer.current);
      openTimer.current = null;
    }
    if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    closeTimer.current = window.setTimeout(() => {
      closeTimer.current = null;
      setState((current) => (current.pinned ? current : CLOSED));
    }, INFO_TIP_CLOSE_GRACE_MS);
  }, []);

  const cancelClose = React.useCallback(() => {
    if (closeTimer.current !== null) {
      window.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
  }, []);

  // A scroll that moves the glyph closes it (Radix Tooltip's rule); a scroll in an unrelated
  // scroller beside it leaves it alone.
  React.useEffect(() => {
    if (!state.open) return;
    const onScroll = (event: Event) => {
      const target = event.target;
      const trigger = triggerRef.current;
      if (trigger && target instanceof Node && !target.contains(trigger)) return;
      close();
    };
    window.addEventListener('scroll', onScroll, { capture: true, passive: true });
    return () => window.removeEventListener('scroll', onScroll, { capture: true });
  }, [state.open, close]);

  const onPointerEnter = (event: React.PointerEvent) => {
    if (event.pointerType === 'touch') return;
    cancelClose();
    if (state.open || openTimer.current !== null) return;
    openTimer.current = window.setTimeout(() => {
      openTimer.current = null;
      setState((current) => (current.open ? current : { open: true, pinned: false }));
    }, INFO_TIP_OPEN_DELAY_MS);
  };

  const onPointerLeave = (event: React.PointerEvent) => {
    if (event.pointerType === 'touch') return;
    scheduleClose();
  };

  const onFocus = () => {
    if (!isFocusFromTabKey()) return;
    clearTimers();
    setState((current) => (current.open ? current : { open: true, pinned: false }));
  };

  const onBlur = () => close();

  const onClick = (event: React.MouseEvent) => {
    // The glyph is its own control: a click on it never reaches a row that navigates or toggles.
    event.stopPropagation();
    clearTimers();
    setState((current) => (current.open && current.pinned ? CLOSED : { open: true, pinned: true }));
  };

  const Trigger: React.ElementType = asChild ? Slot : 'button';
  const triggerProps: Record<string, unknown> = asChild
    ? { tabIndex: 0, className: cn('br-info-tip-target', className) }
    : {
        type: 'button',
        'aria-label': uiCopy.infoTipName(label),
        className: cn('br-info-tip', className),
      };

  return (
    <PopoverPrimitive.Root open={state.open} onOpenChange={(next) => !next && close()}>
      <PopoverPrimitive.Anchor asChild>
        <Trigger
          ref={setRefs}
          {...triggerProps}
          aria-describedby={text ? descriptionId : undefined}
          data-state={state.open ? 'open' : 'closed'}
          data-slot="info-tip-trigger"
          data-testid={testId}
          onPointerEnter={onPointerEnter}
          onPointerLeave={onPointerLeave}
          onFocus={onFocus}
          onBlur={onBlur}
          onClick={onClick}
        >
          {asChild ? children : <Info aria-hidden="true" focusable="false" size={14} />}
        </Trigger>
      </PopoverPrimitive.Anchor>
      {text ? (
        <span id={descriptionId} className="sr-only" data-slot="info-tip-description">
          {text}
        </span>
      ) : null}
      <PopoverPrimitive.Portal>
        <PopoverPrimitive.Content
          data-slot="info-tip-content"
          // The words reach assistive tech through `aria-describedby`; the floating copy is for
          // the eye only, so a screen reader does not hear them twice.
          aria-hidden="true"
          side={side}
          align={align}
          sideOffset={8}
          collisionPadding={8}
          onOpenAutoFocus={(event) => event.preventDefault()}
          onCloseAutoFocus={(event) => event.preventDefault()}
          onInteractOutside={(event) => {
            // A press on the trigger toggles through its own click; it is not "outside".
            const target = event.target;
            if (target instanceof Node && triggerRef.current?.contains(target)) {
              event.preventDefault();
            }
          }}
          onPointerEnter={cancelClose}
          onPointerLeave={(event) => {
            if (event.pointerType !== 'touch') scheduleClose();
          }}
          className={cn(
            TOOLTIP_SURFACE_CLASS_NAME,
            'br-info-tip-content br-tooltip-motion animate-in fade-in-0 data-[state=closed]:animate-out data-[state=closed]:fade-out-0'
          )}
        >
          {text}
        </PopoverPrimitive.Content>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  );
});
