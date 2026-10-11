import * as React from 'react';

import { cn } from '../../utils';

/**
 * The text field, on the control ladder (§3.2).
 *
 * Height is the 32px md rung — the same box a Button, a Select trigger and a
 * menu row take, so a form row and a toolbar cluster line up without anyone
 * measuring. Text inset is 8px, which is what the wrapper construction produces
 * for a field with no leading icon. The value is `text-body` (14/20/400, the
 * role for values, as in `Textarea`) in `--text-default`: the field sets its own
 * ink, so a value never inherits a muted container's and reads as a placeholder
 * (first run, FR-R1-10: the prefilled Endpoint inside the muted tab panel).
 *
 * FILLS BEFORE LINES (owner message 13, R-27, design.md 9.11). The field is a
 * subtle solid fill with no resting outline: the control ground at rest
 * (`--background-fill`), `--background-fill-strong` under the pointer, and the
 * 1px border kept but transparent, so nothing moves when an edge appears. The
 * whole look is authored
 * in `main.css` (`.br-field`, layered so a caller's utility still wins), because
 * the edges it draws are load-bearing: `--border-focus` while focused (D-15: a
 * field firms an edge, never a ring), `--border-danger` while invalid, and the
 * system ink in forced colours, which draw the transparent border. Disabled is
 * the same fill at half opacity.
 *
 * Inside a filled box (a Note, a Card, a dialog's inner panel, a caller's
 * `.br-fill-surface`) the fill steps back to the page ground by itself, so a field
 * never sits fill on fill.
 */
const Input = React.forwardRef<HTMLInputElement, React.ComponentProps<'input'>>(
  ({ className, type, ...props }, ref) => {
    return (
      <input
        type={type}
        className={cn(
          'br-field flex h-control-md w-full rounded-element border px-2 text-body text-text-default transition-[color,background-color,border-color]',
          'file:border-0 file:bg-transparent file:text-label file:text-text-default placeholder:text-text-muted',
          'aria-invalid:border-border-danger disabled:cursor-not-allowed disabled:opacity-50',
          className
        )}
        ref={ref}
        {...props}
      />
    );
  }
);
Input.displayName = 'Input';

export { Input };
