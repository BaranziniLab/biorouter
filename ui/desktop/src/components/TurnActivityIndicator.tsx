import { useElapsedMs } from '../hooks/useElapsedMs';
import { formatElapsed } from '../utils/formatElapsed';
import type { TrailingActivity } from '../utils/trailingActivity';
import { cn } from '../utils';

/** Below this the number flickers in and out on every fast tool boundary. */
export const ELAPSED_REVEAL_MS = 2000;
/** Past this the wait is long enough to deserve an explicit reassurance. */
export const NUDGE_MS = 45000;

interface TurnActivityIndicatorProps {
  activity: TrailingActivity;
  className?: string;
  /**
   * Whether the reader can stop this turn from this tab. False on a delegated
   * subagent's tab in a browser, which has no composer at all (SD-8) — so the
   * nudge must not send the reader to one.
   */
  canStopHere?: boolean;
}

/**
 * The inline "still working" indicator that trails the tool-call block.
 *
 * Distinct from LoadingBioRouter, which narrates the whole turn from a fixed
 * position above the composer: this one lives INSIDE the transcript, directly
 * under the last tool card, because that is where the user is looking while a
 * tool result sits on screen and the next provider round-trip is in flight.
 *
 * All of its inputs are derived (see utils/trailingActivity.ts) and its clock
 * origin comes from the stream store, never from mount time — so it renders
 * identically no matter how many times the message list re-reconciles.
 *
 * Reduced motion is handled globally by the prefers-reduced-motion reset in
 * styles/main.css (DR-13), which nulls animation duration and iteration count
 * for everything; the pulse degrades to a static dot. Do not add a per-component
 * check here — that fights the global rule.
 */
export default function TurnActivityIndicator({
  activity,
  className,
  canStopHere = true,
}: TurnActivityIndicatorProps) {
  const elapsedMs = useElapsedMs(activity.since);
  const showElapsed = elapsedMs !== null && elapsedMs >= ELAPSED_REVEAL_MS;
  const showNudge = elapsedMs !== null && elapsedMs >= NUDGE_MS;
  const elapsedLabel = showElapsed ? formatElapsed(elapsedMs) : null;
  // Screen readers get the time in 15s buckets, not the 1 Hz tick. Below the
  // first full bucket no duration is announced at all — "about 0 seconds" is
  // noise, and rounding a 3s wait up to "about 15 seconds" would be a lie.
  const announcedSeconds =
    elapsedMs !== null && elapsedMs >= 15000 ? Math.floor(elapsedMs / 15000) * 15 : null;

  return (
    <div
      className={cn('w-full animate-fade-slide-up', className)}
      data-testid="turn-activity-indicator"
      data-phase={activity.phase}
    >
      <div
        role="status"
        aria-live="polite"
        aria-atomic="true"
        className="inline-flex items-center gap-2 rounded-full px-1 py-1 text-xs text-text-default/80"
      >
        <span
          aria-hidden="true"
          className="relative flex h-4 w-4 flex-shrink-0 items-center justify-center text-text-default/80"
        >
          <span className="absolute h-4 w-4 rounded-full border border-current animate-[biorouter-working-ring_1.8s_ease-out_infinite]" />
          <span className="absolute h-2.5 w-2.5 rounded-full bg-current opacity-20 animate-[biorouter-working-glow_1.8s_ease-in-out_infinite]" />
          <span className="h-1.5 w-1.5 rounded-full bg-current opacity-70" />
        </span>

        <span className="min-w-0 truncate text-text-muted">{activity.label}</span>

        {/* BR-61: the user's own words, so the press is visibly acknowledged
            with the exact text that left the composer — not just a generic
            spinner they cannot connect to what they typed. */}
        {activity.steerText && (
          <span
            data-testid="turn-activity-steer-text"
            className="min-w-0 max-w-[32ch] truncate rounded-full bg-background-muted px-2 py-0.5 text-text-default/70"
            title={activity.steerText}
          >
            “{activity.steerText}”
          </span>
        )}

        {elapsedLabel && (
          <span
            aria-hidden="true"
            data-testid="turn-activity-elapsed"
            className="flex-shrink-0 tabular-nums text-text-muted/70"
          >
            · {elapsedLabel}
          </span>
        )}

        {/*
          The visible chip ticks once a second and is aria-hidden: inside an
          aria-live region that would be ~60 announcements a minute. This
          sr-only sibling carries the announcement instead, coarsened to 15s
          buckets so a screen reader hears "about 30 seconds elapsed" rather
          than every single second.
        */}
        <span className="sr-only">
          {activity.label}
          {activity.steerText ? `: ${activity.steerText}` : ''}
          {announcedSeconds !== null ? `, about ${announcedSeconds} seconds elapsed` : ''}
        </span>
      </div>

      {showNudge && (
        <div className="pl-7 pt-0.5 text-xs text-text-muted/70">
          {canStopHere
            ? 'Still working. You can stop the turn from the composer.'
            : 'Still working.'}
        </div>
      )}
    </div>
  );
}
