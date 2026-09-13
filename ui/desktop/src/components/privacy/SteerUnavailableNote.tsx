import { cn } from '../../utils';
import { STEER_UNAVAILABLE_REASON, STEER_UNAVAILABLE_SHORT } from './steerUnavailableCopy';
import { isBrowserSurface } from '../../utils/surface';

/**
 * The line that stands where the queue row's "Add now" button would be on a
 * browser-served session (SD-8). See `steerUnavailableCopy.ts` for the ruling
 * and the words.
 *
 * ⚠ **Renders nothing on the desktop**, so a call site can mount it
 * unconditionally — the same contract, and for the same reason, as
 * {@link HostManagedModelNote}: the one call site that forgot an
 * `isBrowserSurface() && …` guard would ship a note telling desktop users that
 * a control they can see working does not work.
 *
 * The shape is `HostManagedModelNote`'s `inset` variant: flush, hairline
 * separated, no card. It sits INSIDE the queue widget, which is itself a
 * bordered strip above the composer, and a rounded box on a strip would read as
 * a second object rather than as a footnote to the row above it.
 */
export function SteerUnavailableNote({
  className,
  short = false,
  testId = 'steer-unavailable-note',
}: {
  /** Layout only: `mt-*`, `mb-*`, `min-w-0`. */
  className?: string;
  /** The one-line form, for a row with no space for the reason. */
  short?: boolean;
  /** Overridable so a surface mounting two of these can tell them apart. */
  testId?: string;
}) {
  if (!isBrowserSurface()) return null;

  return (
    <p
      data-testid={testId}
      className={cn(
        'border-t border-border-subtle px-3 py-2 text-supporting text-text-muted',
        className
      )}
    >
      {short ? STEER_UNAVAILABLE_SHORT : STEER_UNAVAILABLE_REASON}
    </p>
  );
}
