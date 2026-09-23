import * as React from 'react';

import { cn } from '../../utils';

/**
 * The text field, on the control ladder (§3.2).
 *
 * Height is the 32px md rung — the same box a Button, a Select trigger and a
 * menu row take, so a form row and a toolbar cluster line up without anyone
 * measuring. Text inset is 8px, which is what the wrapper construction produces
 * for a field with no leading icon; the type role is `text-label` (14/20/500),
 * the one every control shares.
 *
 * The edge is `--border-emphasized` — ink at 24%, DERIVED from the family's own
 * ink rather than a per-family hex — which is the token the redesign introduced
 * precisely so an interactive border stops being hand-authored per theme.
 *
 * Hover is a whisper, not a colour jump. `inset-ring-2` at 30% of that same edge
 * thickens the boundary from the inside, so nothing shifts and the field does not
 * change hue before you have even committed to it; the old
 * `hover:border-border-strong` swapped one neutral for another and read as a
 * state change the pointer had not earned yet.
 *
 * Focus is deliberately NOT specified here. `main.css` owns it globally for every
 * text field (D-15: focus is a surface shift, never a ring) — the field deepens
 * its own fill and firms its existing edge. All this file does is stand the
 * hover whisper down (`focus:inset-ring-0`) so the two treatments never stack.
 */
const Input = React.forwardRef<HTMLInputElement, React.ComponentProps<'input'>>(
  ({ className, type, ...props }, ref) => {
    return (
      <input
        type={type}
        className={cn(
          'flex h-control-md w-full rounded-element border border-border-emphasized bg-background-default px-2 text-label transition-[color,background-color,border-color,box-shadow]',
          'file:border-0 file:bg-transparent file:text-label file:text-text-default placeholder:text-text-muted',
          'hover:inset-ring-2 hover:inset-ring-border-emphasized/30 focus:inset-ring-0',
          'aria-invalid:border-border-danger disabled:cursor-not-allowed disabled:bg-background-muted disabled:opacity-50',
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
