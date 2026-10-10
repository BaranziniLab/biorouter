import * as React from 'react';

import { cn } from '../../utils';

/**
 * Textarea: the Input recipe for several lines (spec 2.6), the one Crew's
 * `pane/AgentTaskPane.tsx` reproduces by hand.
 *
 * Radius 8, padding 6px 8px, `text-body`, the `--border-control` resting edge (it reads
 * `--border-emphasized` until WS-TOKENS lands the token, through `.br-textarea` in main.css),
 * the hover whisper, and focus owned by `main.css` like every text field. At least 3 rows.
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
        'br-textarea flex w-full rounded-element border border-border-emphasized bg-background-default px-2 py-1.5 text-body text-text-default transition-[color,background-color,border-color,box-shadow]',
        'placeholder:text-text-muted',
        'hover:inset-ring-2 hover:inset-ring-border-emphasized/30 focus:inset-ring-0',
        'aria-invalid:border-border-danger disabled:cursor-not-allowed disabled:bg-background-muted disabled:opacity-50',
        className
      )}
      {...props}
    />
  )
);
Textarea.displayName = 'Textarea';

export { Textarea };
