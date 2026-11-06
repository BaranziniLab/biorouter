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
 * A visible 14px info glyph in subtle ink sits 4px after a label. It follows the app's one rule
 * for a disclosure trigger, "hover peeks, click pins" (docs/desktop-ui/shared-controls.md):
 *   - a PREVIEW opens on pointer hover after 200ms, or at once when the Tab key moves focus onto
 *     it (Tooltip's gate, so a focus a program restores never opens it). It closes on pointer
 *     leave (100ms grace, enough to cross the 8px gap), blur, Escape and an ancestor scroll, and
 *     the trigger carries no mark while it shows;
 *   - a click, tap, Enter or Space KEEPS the very tip the hover previewed, and a second one
 *     closes it. So does a press on the preview itself. A kept tip marks its trigger
 *     (`data-pinned`: the strong fill on the glyph's 20px mark box, a pressed plate on an
 *     `asChild` target) and closes only on Escape, a press outside, that second click, or focus
 *     that really moves elsewhere in the page (Radix's own focus-outside dismissal). Pointer
 *     leave, a scroll and a window switch, which blurs the trigger with nowhere for focus to go,
 *     leave it open.
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

/**
 * The words a label node shows, for an InfoTip's name when the label is not a plain string:
 * strings and numbers, and the `children` of elements, whitespace collapsed. A component's own
 * output is not rendered, so `<OptionalLabel>Initial prompt</OptionalLabel>` reads
 * "Initial prompt" without the "(optional)" it draws. Empty when the node holds no text.
 */
export function plainTextOf(node: React.ReactNode): string {
  const walk = (part: React.ReactNode): string => {
    if (part === null || part === undefined || typeof part === 'boolean') return '';
    if (typeof part === 'string' || typeof part === 'number') return String(part);
    if (Array.isArray(part)) return part.map(walk).join('');
    if (React.isValidElement<{ children?: React.ReactNode }>(part)) {
      return walk(part.props.children);
    }
    return '';
  };
  return walk(node).replace(/\s+/g, ' ').trim();
}

/** A stable id for an InfoTip's description, to share with the control it explains. */
export function useInfoTipId(): string {
  const reactId = React.useId();
  return `br-info-tip-${reactId.replace(/[^a-zA-Z0-9_-]/g, '')}`;
}

/**
 * The things an `asChild` trigger must not sit inside, because each is already the control a
 * pointer or a key addresses there: a button, a link, a label, a menu or listbox row, a tab, a
 * toggle, a field, another InfoTip's trigger. A focusable trigger in one would be a control inside
 * a control: its click would also open the menu or pick the row, and its hint would stack on the
 * host's own (spec 2.6, InfoTip "Never").
 */
export const INFO_TIP_HOST_CONTROL_SELECTOR = [
  'button',
  'a[href]',
  'label',
  'summary',
  'input',
  'select',
  'textarea',
  '[role="button"]',
  '[role="link"]',
  '[role="menuitem"]',
  '[role="menuitemcheckbox"]',
  '[role="menuitemradio"]',
  '[role="option"]',
  '[role="tab"]',
  '[role="treeitem"]',
  '[role="switch"]',
  '[role="checkbox"]',
  '[role="radio"]',
  '[role="combobox"]',
  '[data-slot="info-tip-trigger"]',
].join(', ');

/**
 * For a component that may be drawn inside another control (a badge in a model chip or a picker
 * row): `inside` turns true once its node is found inside one of
 * {@link INFO_TIP_HOST_CONTROL_SELECTOR}, and the component then draws itself without an InfoTip,
 * leaving the words to the control that holds it. Give `notice` to the node as its `ref` in both
 * forms. The first commit draws the InfoTip form and the switch lands before paint, as
 * `BuiltInBadge` does it.
 */
export function useInsideAControl(): {
  inside: boolean;
  notice: (node: HTMLElement | null) => void;
} {
  const [inside, setInside] = React.useState(false);
  const notice = React.useCallback((node: HTMLElement | null) => {
    if (node?.parentElement?.closest(INFO_TIP_HOST_CONTROL_SELECTOR)) setInside(true);
  }, []);
  return { inside, notice };
}

/** An element whose own Enter or Space already clicks it. */
const NATIVELY_ACTIVATED = 'button, a[href], input, select, textarea, summary';

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
const PREVIEW: OpenState = { open: true, pinned: false };
const KEPT: OpenState = { open: true, pinned: true };

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

  // A scroll that moves the glyph closes a preview (Radix Tooltip's rule); a scroll in an
  // unrelated scroller beside it leaves it alone. A kept tip ignores scrolling: the person asked
  // for it to stay, and the popper follows its trigger.
  React.useEffect(() => {
    if (!state.open || state.pinned) return;
    const onScroll = (event: Event) => {
      const target = event.target;
      const trigger = triggerRef.current;
      if (trigger && target instanceof Node && !target.contains(trigger)) return;
      close();
    };
    window.addEventListener('scroll', onScroll, { capture: true, passive: true });
    return () => window.removeEventListener('scroll', onScroll, { capture: true });
  }, [state.open, state.pinned, close]);

  const onPointerEnter = (event: React.PointerEvent) => {
    if (event.pointerType === 'touch') return;
    cancelClose();
    if (state.open || openTimer.current !== null) return;
    openTimer.current = window.setTimeout(() => {
      openTimer.current = null;
      setState((current) => (current.open ? current : PREVIEW));
    }, INFO_TIP_OPEN_DELAY_MS);
  };

  const onPointerLeave = (event: React.PointerEvent) => {
    if (event.pointerType === 'touch') return;
    scheduleClose();
  };

  const onFocus = () => {
    if (!isFocusFromTabKey()) return;
    clearTimers();
    setState((current) => (current.open ? current : PREVIEW));
  };

  // The trigger's blur closes a preview. A kept tip outlives it: switching windows blurs the
  // trigger with nowhere for focus to go, and closed a tip the person had kept. Focus that really
  // moves to something else in the page still closes it, through the content's focus-outside
  // dismissal (`onOpenChange` below), which a window switch never fires. Functional, so a press
  // on the preview (which keeps it) and the blur that press causes settle in order.
  const onBlur = () => {
    clearTimers();
    setState((current) => (current.pinned ? current : CLOSED));
  };

  /** Keep the tip, or close a kept one: the trigger's click, tap, Enter or Space. */
  const toggleKept = () => {
    clearTimers();
    setState((current) => (current.open && current.pinned ? CLOSED : KEPT));
  };

  const onClick = (event: React.MouseEvent) => {
    // The glyph is its own control: a click on it never reaches a row that navigates or toggles.
    event.stopPropagation();
    toggleKept();
  };

  // An `asChild` target is a span with a tab stop, which no key clicks; Enter and Space do here
  // what they do on the glyph's button. A target that is itself a native control keeps its own.
  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key !== 'Enter' && event.key !== ' ') return;
    if (event.currentTarget.matches(NATIVELY_ACTIVATED)) return;
    event.preventDefault();
    event.stopPropagation();
    if (!event.repeat) toggleKept();
  };

  const Trigger: React.ElementType = asChild ? Slot : 'button';
  const triggerProps: Record<string, unknown> = asChild
    ? // `biorouter-focus-region` keeps the base D-15 rule (a `[tabindex]` fill and inset edge)
      // off the target, so the outline in `.br-info-tip-target` is its only focus mark.
      { tabIndex: 0, className: cn('br-info-tip-target biorouter-focus-region', className) }
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
          // Only a kept tip marks its trigger (main.css, the InfoTip rules); a preview leaves it
          // at rest, so the mark always means "this stays until you close it".
          data-pinned={state.open && state.pinned ? 'true' : undefined}
          data-slot="info-tip-trigger"
          data-testid={testId}
          onPointerEnter={onPointerEnter}
          onPointerLeave={onPointerLeave}
          onFocus={onFocus}
          onBlur={onBlur}
          onClick={onClick}
          onKeyDown={asChild ? onKeyDown : undefined}
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
          // A press on the preview keeps it, as a press inside a footer card does: the person
          // reached for the words, and a pointer that then wanders off must not lose them.
          onPointerDown={() => {
            clearTimers();
            setState((current) => (current.open ? KEPT : current));
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
