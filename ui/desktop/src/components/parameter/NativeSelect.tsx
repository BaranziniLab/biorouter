import * as React from 'react';
import { ChevronDown } from '../icons/app-icons';
import { cn } from '../../utils';

/**
 * A native `<select>` on the Input skin (`.br-field`: 32px, radius 8), so a parameter's few
 * fixed choices sit on the same field as every other control in the form: the subtle fill at
 * rest, a transparent 1px edge, the focus edge while focused (R-27). Native rather than the
 * shared react-select, because two fixed four-item lists inside a scrolling dialog need no
 * search and no portal.
 *
 * It names no ground of its own, so it rests on whatever ground its container sets
 * (`--br-control-ground`): the page ground inside a filled parameter card, the fill in a plain
 * dialog. A `bg-*` utility here would beat the layered field rule and leave an unfilled white
 * box.
 *
 * The system arrow is replaced by the app's own 16px stroke-1.5 chevron, the one every Select
 * trigger draws (`ui/Select.tsx`), so a native and a shared select read as the same control.
 * Every prop, including the `id` and `aria-*` a `Field` hands its child, reaches the `<select>`.
 */
export const NativeSelect = React.forwardRef<HTMLSelectElement, React.ComponentProps<'select'>>(
  ({ className, children, ...props }, ref) => (
    <div className="relative">
      <select
        ref={ref}
        className={cn(
          'br-field h-control-md w-full appearance-none rounded-element border pr-8 pl-2 text-label text-text-default',
          className
        )}
        {...props}
      >
        {children}
      </select>
      <ChevronDown
        aria-hidden
        className="pointer-events-none absolute top-1/2 right-2 size-4 -translate-y-1/2 text-text-muted"
      />
    </div>
  )
);
NativeSelect.displayName = 'NativeSelect';
