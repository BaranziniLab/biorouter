import * as React from 'react';

import { cn } from '../../utils';

/**
 * Textarea: the Input recipe for several lines (spec 2.6), the one Crew's
 * `pane/AgentTaskPane.tsx` reproduces by hand.
 *
 * Radius 8, padding 6px 8px, `text-body`, at least 3 rows, and the filled field look shared with
 * Input (`.br-field` in main.css, R-27): `--background-fill` at rest with a transparent 1px border,
 * `--background-fill-strong` under the pointer, the `--border-focus` edge while focused and the
 * `--border-danger` edge while invalid.
 */
export interface TextareaProps extends React.ComponentProps<'textarea'> {
  /** The visible minimum height in rows (at least 3). */
  rows?: number;
}

const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ className, rows = 3, ...props }, ref) => (
    <textarea
      ref={ref}
      rows={Math.max(3, rows)}
      data-slot="textarea"
      className={cn(
        'br-field br-textarea flex w-full rounded-element border px-2 py-1.5 text-body text-text-default transition-[color,background-color,border-color]',
        'placeholder:text-text-muted',
        'aria-invalid:border-border-danger disabled:cursor-not-allowed disabled:opacity-50',
        className
      )}
      {...props}
    />
  )
);
Textarea.displayName = 'Textarea';

export { Textarea };
