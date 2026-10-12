import { cn } from '../../utils';
import { Loader2 } from '../icons/app-icons';

/**
 * Spinner: the one busy glyph (spec 2.6; Crew defect D2 had four spinners at two speeds).
 *
 * A 16px (or 14px) `Loader2` turning once per `--dur-slow × 2` (1,050ms), linear, authored in
 * `main.css` as `.br-spinner` with `@keyframes br-spin`. It stands still under reduced motion.
 * It replaces `animate-spin` across the app, each owner in its own files.
 *
 * Decorative by default (`aria-hidden`): the words beside it say what is busy. Give it a
 * `label` when it stands alone, and it becomes a `role="status"` with that name.
 */
export interface SpinnerProps {
  size?: 14 | 16;
  /** A spoken name, when nothing beside the spinner says what is busy. */
  label?: string;
  className?: string;
  'data-testid'?: string;
}

export function Spinner({ size = 16, label, className, 'data-testid': testId }: SpinnerProps) {
  const glyph = (
    <Loader2
      aria-hidden="true"
      focusable="false"
      size={size}
      data-size={size}
      className={cn('br-spinner', label ? undefined : className)}
      data-testid={label ? undefined : testId}
    />
  );
  if (!label) return glyph;
  return (
    <span
      role="status"
      aria-label={label}
      className={cn('br-spinner-status', className)}
      data-testid={testId}
    >
      {glyph}
    </span>
  );
}
