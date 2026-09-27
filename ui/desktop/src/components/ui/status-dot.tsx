import { cn } from '../../utils';

export type StatusDotTone = 'success' | 'warning' | 'danger' | 'neutral' | 'idle';

export interface StatusDotProps {
  /**
   * `success` connected · `warning` degraded or waiting · `danger` failed ·
   * `neutral` a state with no verdict yet (checking, signing in) · `idle`
   * nothing selected or switched off.
   */
  tone: StatusDotTone;
  /**
   * The dot describes something happening right now. Adds design.md §4.16's 2px
   * halo on a slow period; under reduced motion the halo holds still.
   */
  live?: boolean;
  /**
   * The dot's name when it stands ALONE. With a label it is `role="img"`; without
   * one it is `aria-hidden`, because a dot beside a word ("● Connected") must not
   * make a screen reader say the status twice.
   */
  label?: string;
  /** Layout only (margins, alignment). The fill and the size are the primitive's. */
  className?: string;
}

/**
 * design.md §4.16's status dot: an 8px circle whose fill is a status token.
 *
 * ⚠ **This is not `Dot.tsx`, and it does not replace it.** `Dot` is keyed on a
 * loading status and always carries `role="img"`, which is wrong for a dot that
 * sits beside the word it illustrates. Its callers are untouched; this is the
 * primitive for new surfaces.
 *
 * Everything that paints is authored CSS (`.biorouter-status-dot` in
 * `main.css`), keyed on `data-tone` and `data-live`: the halo needs a
 * pseudo-element, `@keyframes` and a reduced-motion rest, none of which a
 * utility can express, and a newly written utility can silently fail to
 * generate under `BIOROUTER_NO_HMR`.
 *
 * Colour is never the only signal. A caller either puts a word beside the dot or
 * passes `label`.
 */
export function StatusDot({ tone, live = false, label, className }: StatusDotProps) {
  const named = typeof label === 'string' && label.trim().length > 0;
  return (
    <span
      data-slot="status-dot"
      data-tone={tone}
      data-live={live ? 'true' : undefined}
      className={cn('biorouter-status-dot', className)}
      {...(named ? { role: 'img', 'aria-label': label } : { 'aria-hidden': true })}
    />
  );
}
