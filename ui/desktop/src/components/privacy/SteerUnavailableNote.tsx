import { cn } from '../../utils';
import { InfoTip } from '../ui/info-tip';
import {
  STEER_UNAVAILABLE_NAME,
  STEER_UNAVAILABLE_SHORT,
  STEER_UNAVAILABLE_WHY,
} from './steerUnavailableCopy';
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
 *
 * One line wherever it sits, the collapsed row and the open list alike: what
 * cannot happen and the button that does work. Why (the machine cannot tell a
 * person from the model) and where steering works are the info icon beside it
 * (owner, 2026-11-13: one line up front, the detail on hover).
 */
export function SteerUnavailableNote({
  className,
  testId = 'steer-unavailable-note',
}: {
  /** Layout only: `mt-*`, `mb-*`, `min-w-0`. */
  className?: string;
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
      {STEER_UNAVAILABLE_SHORT}{' '}
      <InfoTip label={STEER_UNAVAILABLE_NAME} help={STEER_UNAVAILABLE_WHY} />
    </p>
  );
}
