import * as React from 'react';
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
 * Geometry (redesign spec 2.6): a 16px box, radius 4, a 1.5px `--border-control` edge, inside
 * a 24px hit target, so a checkbox and a radio stacked in one list share their optical axis.
 * Checked fills with the accent and draws a 10px check (stroke 2) in `--text-on-accent`;
 * indeterminate draws an 8×2 bar. The fill eases over `--dur-fast-min`. Everything visual is
 * authored in `main.css` (`.br-checkbox*`), keyed on the input's state, not on new Tailwind
 * strings, because a newly written utility can fail to generate under `BIOROUTER_NO_HMR`.
 * A label sits 8px to the right of the box; the call site places it.
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
      <span className={cn('br-checkbox', className)} data-disabled={disabled ? '' : undefined}>
        <input
          type="checkbox"
          ref={(node) => {
            innerRef.current = node;
            if (typeof forwardedRef === 'function') forwardedRef(node);
            else if (forwardedRef) forwardedRef.current = node;
          }}
          disabled={disabled}
          className={cn(
            'br-checkbox-input peer absolute inset-0 m-0 h-full w-full appearance-none opacity-0',
            !disabled && 'cursor-pointer'
          )}
          {...props}
        />
        {/* The box: `--border-control` at rest (3:1 on the page and card grounds), the accent
            fill when checked or indeterminate, and the focus ring drawn on it, because the
            input it answers to is transparent. */}
        <span aria-hidden className="br-checkbox-box pointer-events-none" />
        {/* The mark is `--text-on-accent`, the one token whose contract is "legible on the
            accent fill". Drawn here rather than through the icon registry, which pins every
            glyph to a 1.5 stroke: at 10px that is under a pixel. */}
        <svg
          aria-hidden
          viewBox="0 0 10 10"
          className="br-checkbox-check pointer-events-none"
          fill="none"
          stroke="currentColor"
          strokeWidth={2}
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M2 5.25 4.25 7.5 8 2.75" />
        </svg>
        <span aria-hidden className="br-checkbox-bar pointer-events-none" />
      </span>
    );
  }
);
Checkbox.displayName = 'Checkbox';

export default Checkbox;
