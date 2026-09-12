import * as React from 'react';
import { Check } from '../icons/app-icons';
import { cn } from '../../utils';

/**
 * The checkbox (§3.3), the last selection control the design system was missing.
 *
 * Chat history shipped a bare `<input type="checkbox">` with an empty
 * `className` — `appearance: auto`, `accent-color: auto` — so it rendered as the
 * macOS system blue in light mode and a bare white square in dark, directly
 * under a page title. The OS control has no idea the app has themes, and no
 * amount of surrounding styling can reach inside it.
 *
 * Built like {@link CustomRadio}, its sibling in the same spec section: a `peer`
 * native input driving styled siblings. That keeps every native behaviour —
 * keyboard, focus, form participation, the label's `htmlFor` association,
 * indeterminate state — without adding a Radix package for one control, and it
 * is why the ring and the tick can both respond to `peer-checked`.
 *
 * ⚠ **The input is the hit target, not an `sr-only` sliver.** It used to be
 * `peer sr-only` — a 1px clipped box in the corner — which made the 22px square
 * everybody can see a picture rather than a control: the only way to toggle it
 * was the keyboard, or a `<label>` that a call site had to remember to provide.
 * Two shipped screens did not (measured 2026-09-11: `ResetPanel`'s per-category
 * boxes toggled on nothing at all, and `ExportAppDialog`'s toggled only on their
 * text, because the label sat beside the box rather than around it).
 *
 * So the fix is here rather than at the call sites: an `appearance-none
 * opacity-0` input stretched over the whole 24px target. It is the topmost
 * thing in the box and every sibling is `pointer-events-none`, so a click
 * anywhere on the square lands on the real input. `appearance-none` plus
 * `opacity-0` is what keeps the OS from drawing its own un-themeable box on top
 * of ours — the reason the original reached for `sr-only`. Wrapping it in a
 * `<label>` still works exactly as before: the browser does not forward a label
 * activation whose target is already the labelled control, so there is no
 * double toggle.
 *
 * The visible square also carries the focus ring now. The global
 * `input[type='checkbox']:focus-visible` rule in `main.css` paints a background
 * and removes the outline, which on a transparent input is no indication at all,
 * so keyboard focus was invisible whichever way the input was hidden.
 *
 * Geometry is the radio's, so a checkbox and a radio stacked in one list agree
 * on their optical axis: a 22px visual box inside a 24px hit target. The outer
 * box is the TARGET and is deliberately 1px larger on each side than what you
 * can see, because a selection control should forgive a near miss without
 * looking heavier for it. The only difference from the radio is the corner —
 * `--radius-inner`, the role the ladder names for "checkbox", against the
 * radio's circle.
 */
export interface CheckboxProps extends Omit<
  React.InputHTMLAttributes<HTMLInputElement>,
  'type' | 'size'
> {
  /** Renders the dash state. Mirrors the DOM property, which has no attribute. */
  indeterminate?: boolean;
  /** Classes for the 24px hit target, not for the hidden input. */
  className?: string;
}

export const Checkbox = React.forwardRef<HTMLInputElement, CheckboxProps>(
  ({ className, indeterminate = false, disabled, ...props }, forwardedRef) => {
    const innerRef = React.useRef<HTMLInputElement | null>(null);

    // `indeterminate` is a property with no HTML attribute, so React cannot set
    // it declaratively — it has to be written to the node.
    React.useEffect(() => {
      if (innerRef.current) innerRef.current.indeterminate = indeterminate;
    }, [indeterminate]);

    return (
      <span
        className={cn(
          'relative inline-flex h-6 w-6 shrink-0 items-center justify-center',
          disabled && 'opacity-50',
          className
        )}
      >
        <input
          type="checkbox"
          ref={(node) => {
            innerRef.current = node;
            if (typeof forwardedRef === 'function') forwardedRef(node);
            else if (forwardedRef) forwardedRef.current = node;
          }}
          disabled={disabled}
          className={cn(
            'peer absolute inset-0 m-0 h-full w-full appearance-none opacity-0',
            !disabled && 'cursor-pointer'
          )}
          {...props}
        />
        {/* `--border-emphasized` is the interactive-border token (§3.2) — ink at
            24%, derived per family — so the unchecked box reads against every
            surface without each family authoring a neutral for it. */}
        <span
          className="pointer-events-none absolute inset-[1px] rounded-inner border-[1.5px] border-border-emphasized
                     transition-colors
                     peer-checked:border-border-accent peer-checked:bg-background-accent
                     peer-indeterminate:border-border-accent peer-indeterminate:bg-background-accent
                     peer-focus-visible:ring-2 peer-focus-visible:ring-border-focus"
        />
        {/* The mark is `--text-on-accent`, not `white`: that is the one token
            whose contract is "legible on the accent fill", which is exactly the
            surface it sits on. A literal white tick is theme-blind — it happens
            to work on Parchment and is a coin toss on any family added later.
            Same reasoning as the switch's thumb. */}
        <Check
          aria-hidden
          className="pointer-events-none relative h-3.5 w-3.5 text-text-on-accent opacity-0
                     transition-opacity
                     peer-checked:opacity-100 peer-indeterminate:opacity-0"
        />
        <span
          aria-hidden
          className="pointer-events-none absolute h-0.5 w-2.5 rounded-full bg-text-on-accent opacity-0
                     transition-opacity
                     peer-indeterminate:opacity-100"
        />
      </span>
    );
  }
);
Checkbox.displayName = 'Checkbox';

export default Checkbox;
